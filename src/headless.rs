use std::{
    io::Write,
    path::{
        Path,
        PathBuf,
    },
    time::Duration,
};

use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            Bind,
            ExportMem,
            Offscreen,
            damage::OutputDamageTracker,
            element::solid::SolidColorRenderElement,
            pixman::PixmanRenderer,
        },
    },
    output::{
        Mode,
        Output,
        PhysicalProperties,
        Subpixel,
    },
    reexports::calloop::{
        EventLoop,
        timer::{
            TimeoutAction,
            Timer,
        },
    },
    utils::{
        Buffer,
        Physical,
        Rectangle,
        Size,
        Transform,
    },
};

use crate::{
    Oxmonad,
    OxmonadData,
    screencopy,
    state::RenderedFrame,
};

const FRAME_INTERVAL: Duration = Duration::from_millis(16);

#[expect(
    clippy::too_many_lines,
    reason = "Headless startup keeps the offscreen output, renderer, timer, and capture wiring \
              together."
)]
pub fn init_headless(
    event_loop: &mut EventLoop<'_, OxmonadData>,
    data: &mut OxmonadData,
) -> Result<(), Box<dyn std::error::Error>> {
    let display_handle = &mut data.display_handle;
    let state = &mut data.state;

    let size = headless_size();
    let mode = Mode {
        size,
        refresh: 60_000,
    };

    let output = Output::new(
        "headless".to_string(),
        PhysicalProperties {
            size: (0, 0).into(),
            subpixel: Subpixel::Unknown,
            make: "Oxmonad".into(),
            model: "Headless".into(),
        },
    );
    let _global = output.create_global::<Oxmonad>(display_handle);
    output.change_current_state(
        Some(mode),
        Some(Transform::Normal),
        None,
        Some((0, 0).into()),
    );
    output.set_preferred(mode);

    state.space.map_output(&output, (0, 0));

    // SAFETY: set during single-threaded startup, before any client is spawned.
    #[expect(
        unsafe_code,
        reason = "`std::env::set_var` is unsafe as of edition 2024; this runs on the main thread \
                  during startup before any threads or clients exist."
    )]
    unsafe {
        std::env::set_var("WAYLAND_DISPLAY", &state.socket_name);
    }

    let mut renderer = PixmanRenderer::new()?;
    let mut buffer = renderer.create_buffer(
        Fourcc::Argb8888,
        Size::<i32, Buffer>::from((size.w, size.h)),
    )?;
    let mut damage_tracker = OutputDamageTracker::from_output(&output);
    let mut frame_capture = FrameCapture::from_env();
    let mut script = HeadlessScript::from_env();
    let mut rendered_frames = 0_u64;

    event_loop
        .handle()
        .insert_source(Timer::immediate(), move |_, (), data| {
            let display = &mut data.display_handle;
            let state = &mut data.state;
            rendered_frames = rendered_frames.saturating_add(1);
            script.apply_if_ready(state);

            let scale = output.current_scale().fractional_scale();
            let mut overlays = state.bar_elements(&output, scale);
            overlays.extend(state.border_elements(scale));

            let frame = {
                let mut framebuffer = match renderer.bind(&mut buffer) {
                    Ok(framebuffer) => framebuffer,
                    Err(err) => {
                        tracing::error!(%err, "failed to bind headless renderer");
                        state.loop_signal.stop();
                        return TimeoutAction::Drop;
                    }
                };

                if let Err(err) = smithay::desktop::space::render_output::<
                    PixmanRenderer,
                    SolidColorRenderElement,
                    _,
                    _,
                >(
                    &output,
                    &mut renderer,
                    &mut framebuffer,
                    1.0,
                    0,
                    [&state.space],
                    &overlays,
                    &mut damage_tracker,
                    [0.1, 0.1, 0.1, 1.0],
                ) {
                    tracing::error!(%err, "failed to render headless frame");
                    state.loop_signal.stop();
                    return TimeoutAction::Drop;
                }

                capture_last_frame(&mut renderer, &framebuffer, size)
            };

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

            if let Some(capture) = frame_capture.as_mut()
                && capture.should_capture(rendered_frames)
            {
                if let Err(err) = capture.write(frame.as_ref()) {
                    tracing::error!(%err, "failed to write headless frame capture");
                }
                state.loop_signal.stop();
                return TimeoutAction::Drop;
            }

            TimeoutAction::ToDuration(FRAME_INTERVAL)
        })?;

    Ok(())
}

fn headless_size() -> Size<i32, Physical> {
    let width = std::env::var("OXMONAD_HEADLESS_WIDTH")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(1000);
    let height = std::env::var("OXMONAD_HEADLESS_HEIGHT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(1000);

    Size::from((width, height))
}

#[derive(Debug)]
struct HeadlessScript {
    expected_windows: Option<usize>,
    actions: Vec<HeadlessAction>,
    applied: bool,
}

impl HeadlessScript {
    fn from_env() -> Self {
        let expected_windows = std::env::var("OXMONAD_HEADLESS_EXPECT_WINDOWS")
            .ok()
            .and_then(|value| value.parse().ok());
        let actions = std::env::var("OXMONAD_HEADLESS_ACTIONS")
            .map(|value| {
                value
                    .split(',')
                    .filter_map(HeadlessAction::parse)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        Self {
            expected_windows,
            actions,
            applied: false,
        }
    }

    fn apply_if_ready(&mut self, state: &mut Oxmonad) {
        if self.applied || self.actions.is_empty() {
            return;
        }

        if let Some(expected_windows) = self.expected_windows
            && state.windows.len() < expected_windows
        {
            return;
        }

        for action in &self.actions {
            action.apply(state);
        }
        self.applied = true;
    }
}

#[derive(Clone, Copy, Debug)]
enum HeadlessAction {
    FocusNext,
    FocusPrev,
    SwapMaster,
}

impl HeadlessAction {
    fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "focus-next" => Some(Self::FocusNext),
            "focus-prev" => Some(Self::FocusPrev),
            "swap-master" => Some(Self::SwapMaster),
            unknown if !unknown.is_empty() => {
                tracing::warn!(%unknown, "ignoring unknown headless action");
                None
            }
            _ => None,
        }
    }

    fn apply(self, state: &mut Oxmonad) {
        match self {
            Self::FocusNext => state.focus_next(),
            Self::FocusPrev => state.focus_prev(),
            Self::SwapMaster => state.swap_master(),
        }
    }
}

#[derive(Debug)]
struct FrameCapture {
    path: PathBuf,
    after_frames: u64,
    captured: bool,
}

impl FrameCapture {
    fn from_env() -> Option<Self> {
        let path = std::env::var_os("OXMONAD_CAPTURE_FRAME")?;
        let after_frames = std::env::var("OXMONAD_CAPTURE_AFTER_FRAMES")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(60);

        Some(Self {
            path: path.into(),
            after_frames,
            captured: false,
        })
    }

    fn should_capture(&self, rendered_frames: u64) -> bool {
        !self.captured && rendered_frames >= self.after_frames
    }

    fn write(&mut self, frame: Option<&RenderedFrame>) -> std::io::Result<()> {
        self.captured = true;
        let Some(frame) = frame else {
            return Err(std::io::Error::other("no frame was captured"));
        };

        write_frame_capture(&self.path, frame)
    }
}

fn capture_last_frame(
    renderer: &mut PixmanRenderer,
    framebuffer: &<PixmanRenderer as smithay::backend::renderer::RendererSuper>::Framebuffer<'_>,
    size: Size<i32, Physical>,
) -> Option<RenderedFrame> {
    let buffer_size = Size::<i32, Buffer>::from((size.w, size.h));
    let region = Rectangle::from_size(buffer_size);
    let mapping = renderer
        .copy_framebuffer(framebuffer, region, Fourcc::Argb8888)
        .ok()?;
    let bytes = renderer.map_texture(&mapping).ok()?;

    Some(RenderedFrame {
        size: buffer_size,
        stride: usize::try_from(buffer_size.w).ok()? * 4,
        pixels: bytes.to_vec(),
    })
}

fn write_frame_capture(path: &Path, frame: &RenderedFrame) -> std::io::Result<()> {
    let mut file = std::fs::File::create(path)?;
    writeln!(
        file,
        "OXMONAD_ARGB8888 {} {} {}",
        frame.size.w, frame.size.h, frame.stride
    )?;
    file.write_all(&frame.pixels)
}
