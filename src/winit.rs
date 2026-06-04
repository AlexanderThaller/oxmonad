use std::time::Duration;

use smithay::{
    backend::{
        renderer::{
            damage::OutputDamageTracker,
            element::solid::SolidColorRenderElement,
            gles::GlesRenderer,
        },
        winit::{
            self,
            WinitEvent,
        },
    },
    output::{
        Mode,
        Output,
        PhysicalProperties,
        Subpixel,
    },
    reexports::calloop::EventLoop,
    utils::{
        Rectangle,
        Transform,
    },
};

use crate::{
    Oxmonad,
    OxmonadData,
    screencopy,
};

#[expect(
    clippy::too_many_lines,
    reason = "Backend startup keeps output setup, environment, and the redraw/input event handler \
              together, mirroring the other backends' single init function."
)]
pub fn init_winit(
    event_loop: &mut EventLoop<'_, OxmonadData>,
    data: &mut OxmonadData,
) -> Result<(), Box<dyn std::error::Error>> {
    let display_handle = &mut data.display_handle;
    let state = &mut data.state;

    let (mut backend, winit) = winit::init()?;

    let mode = Mode {
        size: backend.window_size(),
        refresh: 60_000,
    };

    let output = Output::new(
        "winit".to_string(),
        PhysicalProperties {
            size: (0, 0).into(),
            subpixel: Subpixel::Unknown,
            make: "Smithay".into(),
            model: "Winit".into(),
        },
    );
    let _global = output.create_global::<Oxmonad>(display_handle);
    output.change_current_state(
        Some(mode),
        Some(Transform::Flipped180),
        None,
        Some((0, 0).into()),
    );
    output.set_preferred(mode);

    state.space.map_output(&output, (0, 0));

    let mut damage_tracker = OutputDamageTracker::from_output(&output);

    // SAFETY: set during single-threaded startup, before any client is spawned.
    #[expect(
        unsafe_code,
        reason = "`std::env::set_var` is unsafe as of edition 2024; this runs on the main thread \
                  during startup before any threads or clients exist."
    )]
    unsafe {
        std::env::set_var("WAYLAND_DISPLAY", &state.socket_name);
    }

    event_loop
        .handle()
        .insert_source(winit, move |event, (), data| {
            let display = &mut data.display_handle;
            let state = &mut data.state;

            match event {
                WinitEvent::Resized { size, .. } => {
                    output.change_current_state(
                        Some(Mode {
                            size,
                            refresh: 60_000,
                        }),
                        None,
                        None,
                        None,
                    );
                    // Re-tile to the new output size.
                    state.relayout();
                }
                WinitEvent::Input(event) => state.process_input_event(event),
                WinitEvent::Redraw => {
                    let size = backend.window_size();
                    let damage = Rectangle::from_size(size);

                    // Compositor UI is drawn as solid-color elements on top of
                    // the windows: the built-in bar plus window borders.
                    let scale = output.current_scale().fractional_scale();
                    let mut overlays = state.bar_elements(&output, scale);
                    overlays.extend(state.border_elements(scale));

                    {
                        let Ok((renderer, mut framebuffer)) = backend.bind() else {
                            tracing::error!("failed to bind winit renderer");
                            state.loop_signal.stop();
                            return;
                        };

                        if let Err(err) = smithay::desktop::space::render_output::<
                            GlesRenderer,
                            SolidColorRenderElement,
                            _,
                            _,
                        >(
                            &output,
                            renderer,
                            &mut framebuffer,
                            1.0,
                            0,
                            [&state.space],
                            &overlays,
                            &mut damage_tracker,
                            [0.1, 0.1, 0.1, 1.0],
                        ) {
                            tracing::error!(%err, "failed to render winit frame");
                            state.loop_signal.stop();
                            return;
                        }
                    }
                    if let Err(err) = backend.submit(Some(&[damage])) {
                        tracing::error!(%err, "failed to submit winit frame");
                        state.loop_signal.stop();
                        return;
                    }
                    screencopy::finish_pending_screencopies_for_output(state, &output, scale);

                    state.space.elements().for_each(|window| {
                        window.send_frame(
                            &output,
                            state.start_time.elapsed(),
                            Some(Duration::ZERO),
                            |_, _| Some(output.clone()),
                        );
                    });

                    state.space.refresh();
                    state.popups.cleanup();
                    let _ = display.flush_clients();

                    // Ask for redraw to schedule new frame.
                    backend.window().request_redraw();
                }
                WinitEvent::CloseRequested => {
                    state.loop_signal.stop();
                }
                WinitEvent::Focus(_) => (),
            }
        })?;

    Ok(())
}
