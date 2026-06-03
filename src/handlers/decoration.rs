//! `xdg-decoration` support.
//!
//! Implementing this protocol lets us tell clients to use *server-side*
//! decorations. oxmonad draws no decorations of its own (only the tiling
//! border), so the effect is plain, chrome-less windows: no titlebar and no
//! close button, the way xmonad presents tiled windows.

use smithay::{
    delegate_xdg_decoration,
    reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode,
    wayland::shell::xdg::{
        ToplevelSurface,
        decoration::XdgDecorationHandler,
    },
};

use crate::Oxmonad;

impl XdgDecorationHandler for Oxmonad {
    fn new_decoration(&mut self, toplevel: ToplevelSurface) {
        force_server_side(&toplevel);
    }

    fn request_mode(&mut self, toplevel: ToplevelSurface, _mode: Mode) {
        // Ignore the client's preference: we always want server-side (i.e. no)
        // decorations.
        force_server_side(&toplevel);
    }

    fn unset_mode(&mut self, toplevel: ToplevelSurface) {
        force_server_side(&toplevel);
    }
}

/// Advertises server-side decorations for `toplevel`.
fn force_server_side(toplevel: &ToplevelSurface) {
    toplevel.with_pending_state(|state| {
        state.decoration_mode = Some(Mode::ServerSide);
    });
    toplevel.send_pending_configure();
}

delegate_xdg_decoration!(Oxmonad);
