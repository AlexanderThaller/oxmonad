//! oxmonad: an xmonad-inspired Wayland compositor built on Smithay.
//!
//! This is the initial scaffold: a minimal compositor based on Smithay's
//! `smallvil` example. It runs nested as a window inside an existing X11 or
//! Wayland session via the winit backend, which makes it easy to develop and
//! test without leaving your current desktop.
//!
//! The tiling/window-management logic that makes this "xmonad-like" is not here
//! yet; this just gets a working compositor on screen that can host clients.
#![allow(
    irrefutable_let_patterns,
    reason = "Backend selection and some `if let` chains stay written as `let` patterns so new \
              fallible variants can be added without reworking the control flow."
)]

mod bar;
mod config;
mod cursor;
mod floating;
mod grabs;
mod handlers;
mod headless;
mod input;
mod keybindings;
mod layout;
mod process;
mod prompt;
mod rules;
mod screencopy;
mod snapshot;
mod state;
mod udev;
mod winit;
mod workspaces;

use smithay::reexports::{
    calloop::EventLoop,
    wayland_server::{
        Display,
        DisplayHandle,
    },
};
pub use state::Oxmonad;

/// Shared data threaded through the calloop event loop callbacks.
#[expect(
    missing_debug_implementations,
    reason = "Holds Smithay state and a `DisplayHandle`, neither of which implement `Debug`."
)]
pub struct OxmonadData {
    state: Oxmonad,
    display_handle: DisplayHandle,
    /// Backend state for the TTY/DRM backend. `None` for the winit and headless
    /// backends, which keep their state inside their calloop closures. The DRM
    /// backend has several event sources (session, libinput, udev, DRM page
    /// flips) that all need shared access to the GPU state, so it lives here.
    tty: Option<udev::TtyState>,
}

/// Which backend oxmonad should drive.
enum Backend {
    /// Render to the GPU on a bare VT (no host compositor), like weston/sway.
    Tty,
    /// Run nested as a window inside an existing X11/Wayland session.
    Winit,
    /// Offscreen rendering for snapshot/integration tests.
    Headless,
}

impl Backend {
    /// Pick a backend from `OXMONAD_BACKEND`, falling back to auto-detection.
    ///
    /// When unset, we run nested (winit) if a host display is available
    /// (`WAYLAND_DISPLAY`/`DISPLAY` set), otherwise we assume a bare VT and use
    /// the TTY/DRM backend.
    fn from_env() -> Self {
        match std::env::var("OXMONAD_BACKEND").as_deref() {
            Ok("headless") => Self::Headless,
            Ok("winit") => Self::Winit,
            Ok("tty" | "drm" | "udev") => Self::Tty,
            _ => {
                let has_host_display = std::env::var_os("WAYLAND_DISPLAY").is_some()
                    || std::env::var_os("DISPLAY").is_some();
                if has_host_display {
                    Self::Winit
                } else {
                    Self::Tty
                }
            }
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    if let Ok(env_filter) = tracing_subscriber::EnvFilter::try_from_default_env() {
        tracing_subscriber::fmt().with_env_filter(env_filter).init();
    } else {
        tracing_subscriber::fmt().init();
    }

    let mut event_loop: EventLoop<'_, OxmonadData> = EventLoop::try_new()?;

    let display: Display<Oxmonad> = Display::new()?;
    let display_handle = display.handle();
    let state = Oxmonad::new(&mut event_loop, display);

    let mut data = OxmonadData {
        state,
        display_handle,
        tty: None,
    };

    match Backend::from_env() {
        Backend::Headless => crate::headless::init_headless(&mut event_loop, &mut data)?,
        Backend::Winit => crate::winit::init_winit(&mut event_loop, &mut data)?,
        Backend::Tty => crate::udev::init_tty(&mut event_loop, &mut data)?,
    }

    eprintln!(
        "OXMONAD_WAYLAND_DISPLAY={}",
        data.state.socket_name.to_string_lossy()
    );

    event_loop.run(None, &mut data, move |_| {
        // oxmonad is running
    })?;

    Ok(())
}
