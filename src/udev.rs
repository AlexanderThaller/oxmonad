//! TTY/DRM backend: run oxmonad directly on a bare virtual terminal, driving
//! the GPU through DRM/KMS — the way weston and sway run — instead of nested
//! inside another compositor (see [`crate::winit`]).
//!
//! This is a deliberately minimal, single-GPU backend aimed at running in a VM
//! or on a simple machine:
//!
//! * [`LibSeatSession`] manages session/seat ownership and VT switching, and
//!   hands us authenticated file descriptors for the GPU and input devices.
//! * libinput (via [`LibinputInputBackend`]) feeds keyboard/pointer events into
//!   the same [`Oxmonad::process_input_event`] path the winit backend uses.
//! * The primary GPU is opened as a [`DrmDevice`] with a [`GbmDevice`]
//!   allocator and an EGL/GLES renderer ([`GlesRenderer`]); each connected
//!   output gets a [`DrmCompositor`] that scans out our frames.
//!
//! Rendering is driven by a periodic timer (like [`crate::headless`]); page
//! flips are completed on the DRM `VBlank` event. Monitor hotplug and
//! multi-GPU setups are intentionally not handled here.

use std::time::Duration;

use smithay::{
    backend::{
        allocator::{
            Fourcc,
            gbm::{
                GbmAllocator,
                GbmBufferFlags,
                GbmDevice,
            },
        },
        drm::{
            DrmDevice,
            DrmDeviceFd,
            DrmEvent,
            compositor::{
                DrmCompositor,
                FrameFlags,
            },
            exporter::gbm::GbmFramebufferExporter,
        },
        egl::{
            EGLContext,
            EGLDisplay,
        },
        libinput::{
            LibinputInputBackend,
            LibinputSessionInterface,
        },
        renderer::{
            element::{
                solid::SolidColorRenderElement,
                surface::WaylandSurfaceRenderElement,
            },
            gles::GlesRenderer,
        },
        session::{
            Event as SessionEvent,
            Session,
            libseat::LibSeatSession,
        },
        udev::primary_gpu,
    },
    desktop::space::SpaceRenderElements,
    output::{
        Mode,
        Output,
        PhysicalProperties,
        Subpixel,
    },
    reexports::{
        calloop::{
            EventLoop,
            timer::{
                TimeoutAction,
                Timer,
            },
        },
        drm::control::{
            Device as ControlDevice,
            ModeTypeFlags,
            ResourceHandles,
            connector,
            crtc,
        },
        input::Libinput,
        rustix::fs::OFlags,
        wayland_server::backend::GlobalId,
    },
    utils::{
        DeviceFd,
        Transform,
    },
};

use crate::{
    Oxmonad,
    OxmonadData,
    screencopy,
};

/// Background colour cleared behind all windows (matches the winit backend).
const CLEAR_COLOR: [f32; 4] = [0.1, 0.1, 0.1, 1.0];

/// How often we attempt to render. Page flips complete on `VBlank`, but a
/// timer keeps us responsive to client damage when no flip is in flight.
const FRAME_INTERVAL: Duration = Duration::from_millis(16);

/// The concrete [`DrmCompositor`] instantiation used by this backend: a GBM
/// swapchain on the primary GPU, no per-frame user data.
type GbmDrmCompositor =
    DrmCompositor<GbmAllocator<DrmDeviceFd>, GbmFramebufferExporter<DrmDeviceFd>, (), DrmDeviceFd>;

smithay::backend::renderer::element::render_elements! {
    /// Everything composited onto a DRM output: the managed windows from the
    /// space plus our solid-colour UI overlays (bar and window borders). This
    /// mirrors Smithay's private `OutputRenderElements`, specialised to the
    /// [`GlesRenderer`] this backend uses and the
    /// [`WaylandSurfaceRenderElement`] window element type.
    OxElement<='a, GlesRenderer, C>;
    Space = SpaceRenderElements<GlesRenderer, WaylandSurfaceRenderElement<GlesRenderer>>,
    Custom = &'a C,
}

/// State for the TTY/DRM backend, stored in [`OxmonadData::tty`] so the
/// session, input, and page-flip event sources can all reach it.
pub struct TtyState {
    /// Owns the libseat session for the lifetime of the backend. Held so the
    /// seat is not released while we run (and as the handle for future VT
    /// switching), even though we currently react to session changes through
    /// the notifier rather than calling into it.
    #[expect(
        dead_code,
        reason = "RAII guard for the libseat session; kept alive intentionally."
    )]
    session: LibSeatSession,
    drm: DrmDevice,
    // The renderer owns an EGL display backed by a clone of the GBM device, so
    // it keeps the GPU device alive on its own; no separate `gbm` field needed.
    renderer: GlesRenderer,
    libinput: Libinput,
    surfaces: Vec<Surface>,
}

/// One scanned-out output: a connector/CRTC pair and its compositor.
struct Surface {
    crtc: crtc::Handle,
    output: Output,
    compositor: GbmDrmCompositor,
    /// Keeps the `wl_output` global registered; dropping it unadvertises the
    /// output to clients.
    _global: GlobalId,
}

/// Bring up the TTY/DRM backend and register its event sources on the loop.
#[expect(
    clippy::too_many_lines,
    reason = "Backend startup keeps session, GPU, renderer, output scanning, and event-source \
              wiring together, mirroring the other backends' single init function."
)]
pub fn init_tty(
    event_loop: &mut EventLoop<'_, OxmonadData>,
    data: &mut OxmonadData,
) -> Result<(), Box<dyn std::error::Error>> {
    let loop_handle = event_loop.handle();

    // 1. Take the session/seat: this gives us VT ownership and the ability to open
    //    the GPU and input devices without root.
    let (mut session, session_notifier) = LibSeatSession::new()?;
    let seat_name = session.seat();

    // 2. Input via libinput, sharing the session so device fds are opened through
    //    libseat.
    let mut libinput = Libinput::new_with_udev(LibinputSessionInterface::from(session.clone()));
    libinput
        .udev_assign_seat(&seat_name)
        .map_err(|()| "failed to assign libinput udev seat")?;
    let libinput_backend = LibinputInputBackend::new(libinput.clone());

    // 3. Find and open the primary GPU.
    let primary_path =
        primary_gpu(&seat_name)?.ok_or("no GPU found for this seat; is a DRM device present?")?;
    tracing::info!(path = ?primary_path, seat = %seat_name, "opening primary GPU");

    let fd = session.open(
        &primary_path,
        OFlags::RDWR | OFlags::CLOEXEC | OFlags::NONBLOCK,
    )?;
    let device_fd = DrmDeviceFd::new(DeviceFd::from(fd));
    let (mut drm, drm_notifier) = DrmDevice::new(device_fd.clone(), true)?;
    let gbm = GbmDevice::new(device_fd)?;

    // 4. EGL/GLES renderer on top of the GBM device.
    #[expect(
        unsafe_code,
        reason = "`EGLDisplay::new` is unsafe; the backing `GbmDevice` is kept alive in \
                  `TtyState` for as long as the renderer."
    )]
    let egl_display = unsafe { EGLDisplay::new(gbm.clone())? };
    let egl_context = EGLContext::new(&egl_display)?;
    #[expect(
        unsafe_code,
        reason = "`GlesRenderer::new` is unsafe; the EGL context is created above and owned by \
                  the renderer."
    )]
    let renderer = unsafe { GlesRenderer::new(egl_context)? };
    let render_formats = renderer.egl_context().dmabuf_render_formats().clone();

    // 5. Scan connectors and build one compositor per connected output.
    let resources: ResourceHandles = drm.resource_handles()?;
    let mut surfaces = Vec::new();
    let mut used_crtcs: Vec<crtc::Handle> = Vec::new();
    let mut x_offset = 0;

    for connector_handle in resources.connectors() {
        let Ok(connector_info) = drm.get_connector(*connector_handle, false) else {
            continue;
        };
        if connector_info.state() != connector::State::Connected {
            continue;
        }

        let Some(drm_mode) = preferred_mode(&connector_info) else {
            tracing::warn!(connector = ?connector_handle, "connected output has no modes; skipping");
            continue;
        };
        let Some(crtc) = pick_crtc(&drm, &resources, &connector_info, &used_crtcs) else {
            tracing::warn!(connector = ?connector_handle, "no free CRTC for output; skipping");
            continue;
        };
        used_crtcs.push(crtc);

        let (width, height) = drm_mode.size();
        let mode = Mode {
            size: (i32::from(width), i32::from(height)).into(),
            refresh: i32::try_from(drm_mode.vrefresh()).unwrap_or(60) * 1000,
        };
        let (phys_w, phys_h) = connector_info.size().unwrap_or((0, 0));

        let output = Output::new(
            format!("DRM-{connector_handle:?}"),
            PhysicalProperties {
                size: (
                    i32::try_from(phys_w).unwrap_or(0),
                    i32::try_from(phys_h).unwrap_or(0),
                )
                    .into(),
                subpixel: Subpixel::Unknown,
                make: "oxmonad".into(),
                model: "DRM".into(),
            },
        );
        let global = output.create_global::<Oxmonad>(&data.display_handle);
        output.change_current_state(
            Some(mode),
            Some(Transform::Normal),
            None,
            Some((x_offset, 0).into()),
        );
        output.set_preferred(mode);
        data.state.space.map_output(&output, (x_offset, 0));
        x_offset += i32::from(width);

        let drm_surface = drm.create_surface(crtc, drm_mode, &[*connector_handle])?;
        let allocator = GbmAllocator::new(
            gbm.clone(),
            GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT,
        );
        let compositor = DrmCompositor::new(
            &output,
            drm_surface,
            None,
            allocator,
            GbmFramebufferExporter::new(gbm.clone(), None),
            [Fourcc::Argb8888, Fourcc::Xrgb8888],
            render_formats.clone(),
            (64, 64).into(),
            Some(gbm.clone()),
        )?;

        tracing::info!(output = %output.name(), ?mode, "DRM output online");
        surfaces.push(Surface {
            crtc,
            output,
            compositor,
            _global: global,
        });
    }

    if surfaces.is_empty() {
        return Err("no connected outputs found on the primary GPU".into());
    }

    data.state.relayout();

    // SAFETY: set during single-threaded startup, before any client is spawned.
    #[expect(
        unsafe_code,
        reason = "`std::env::set_var` is unsafe as of edition 2024; this runs on the main thread \
                  during startup before any threads or clients exist."
    )]
    unsafe {
        std::env::set_var("WAYLAND_DISPLAY", &data.state.socket_name);
    }

    // Publish our session environment so user services and portals can reach
    // us. Safe here because the TTY backend means oxmonad *is* the session.
    crate::process::import_environment();

    data.tty = Some(TtyState {
        session,
        drm,
        renderer,
        libinput,
        surfaces,
    });

    // 6. Wire event sources.

    // Page flips: complete the in-flight frame so the swapchain can recycle.
    loop_handle.insert_source(drm_notifier, move |event, _metadata, data| match event {
        DrmEvent::VBlank(crtc) => {
            if let Some(tty) = data.tty.as_mut() {
                for surface in &mut tty.surfaces {
                    if surface.crtc == crtc
                        && let Err(err) = surface.compositor.frame_submitted()
                    {
                        tracing::warn!(?err, "frame_submitted failed");
                    }
                }
            }
        }
        DrmEvent::Error(err) => tracing::error!(?err, "DRM error"),
    })?;

    // Input: feed libinput events into the shared input handler.
    loop_handle.insert_source(libinput_backend, move |event, (), data| {
        data.state.process_input_event(event);
    })?;

    // Session: handle VT switches (pause/resume of devices).
    loop_handle.insert_source(session_notifier, move |event, (), data| {
        let Some(tty) = data.tty.as_mut() else {
            return;
        };
        match event {
            SessionEvent::PauseSession => {
                tty.libinput.suspend();
                tty.drm.pause();
            }
            SessionEvent::ActivateSession => {
                if let Err(err) = tty.libinput.resume() {
                    tracing::error!(?err, "failed to resume libinput");
                }
                if let Err(err) = tty.drm.activate(false) {
                    tracing::error!(?err, "failed to reactivate DRM device");
                }
                // Force a full redraw on the next tick.
                for surface in &mut tty.surfaces {
                    surface.compositor.reset_buffers();
                }
            }
        }
    })?;

    // Render loop.
    loop_handle.insert_source(Timer::immediate(), move |_, (), data| {
        let OxmonadData {
            state,
            display_handle,
            tty,
        } = data;
        if let Some(TtyState {
            renderer, surfaces, ..
        }) = tty.as_mut()
        {
            for surface in surfaces.iter_mut() {
                render_surface(state, renderer, surface);
            }
            state.space.refresh();
            state.popups.cleanup();
            let _ = display_handle.flush_clients();
        }
        TimeoutAction::ToDuration(FRAME_INTERVAL)
    })?;

    Ok(())
}

/// Render and queue a single frame for one output.
fn render_surface(state: &mut Oxmonad, renderer: &mut GlesRenderer, surface: &mut Surface) {
    let output = surface.output.clone();
    let scale = output.current_scale().fractional_scale();

    // Compositor UI (bar + window borders) drawn on top of the windows, same as
    // the winit backend.
    let mut overlays = state.bar_elements(&output, scale);
    overlays.extend(state.border_elements(scale));

    let space_elements = match state
        .space
        .render_elements_for_output(renderer, &output, 1.0)
    {
        Ok(elements) => elements,
        Err(err) => {
            tracing::error!(?err, "failed to collect space render elements");
            return;
        }
    };

    let mut elements: Vec<OxElement<'_, SolidColorRenderElement>> =
        Vec::with_capacity(overlays.len() + space_elements.len());
    elements.extend(overlays.iter().map(OxElement::Custom));
    elements.extend(space_elements.into_iter().map(OxElement::Space));

    match surface
        .compositor
        .render_frame(renderer, &elements, CLEAR_COLOR, FrameFlags::DEFAULT)
    {
        Ok(frame) => {
            if !frame.is_empty
                && let Err(err) = surface.compositor.queue_frame(())
            {
                tracing::error!(?err, "failed to queue DRM frame");
            }
        }
        // During a VT switch the surface is inactive; that is expected.
        Err(err) => tracing::debug!(?err, "render_frame skipped"),
    }

    // Tell clients they may draw the next frame.
    state.space.elements().for_each(|window| {
        window.send_frame(
            &output,
            state.start_time.elapsed(),
            Some(Duration::ZERO),
            |_, _| Some(output.clone()),
        );
    });

    screencopy::finish_pending_screencopies_for_output(state, &output, scale);
}

/// Pick the connector's preferred mode, falling back to its first mode.
fn preferred_mode(connector: &connector::Info) -> Option<smithay::reexports::drm::control::Mode> {
    connector
        .modes()
        .iter()
        .find(|mode| mode.mode_type().contains(ModeTypeFlags::PREFERRED))
        .or_else(|| connector.modes().first())
        .copied()
}

/// Find a CRTC that can drive `connector` and is not already in use.
fn pick_crtc(
    drm: &DrmDevice,
    resources: &ResourceHandles,
    connector: &connector::Info,
    used: &[crtc::Handle],
) -> Option<crtc::Handle> {
    connector
        .encoders()
        .iter()
        .filter_map(|encoder_handle| drm.get_encoder(*encoder_handle).ok())
        .flat_map(|encoder| resources.filter_crtcs(encoder.possible_crtcs()))
        .find(|crtc| !used.contains(crtc))
}
