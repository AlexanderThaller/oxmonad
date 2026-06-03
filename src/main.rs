//! oxmonad: an xmonad-inspired Wayland compositor built on Smithay.
//!
//! This is the initial scaffold: a minimal compositor based on Smithay's
//! `smallvil` example. It runs nested as a window inside an existing X11 or
//! Wayland session via the winit backend, which makes it easy to develop and
//! test without leaving your current desktop.
//!
//! The tiling/window-management logic that makes this "xmonad-like" is not here
//! yet; this just gets a working compositor on screen that can host clients.
#![allow(irrefutable_let_patterns)]

mod config;
mod grabs;
mod handlers;
mod input;
mod keybindings;
mod layout;
mod process;
mod screencopy;
#[cfg(test)]
mod snapshot;
mod state;
mod winit;

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
    };

    crate::winit::init_winit(&mut event_loop, &mut data)?;

    crate::process::spawn_shell(crate::config::WAYBAR, "status bar");

    event_loop.run(None, &mut data, move |_| {
        // oxmonad is running
    })?;

    Ok(())
}
