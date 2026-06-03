//! Keyboard shortcut handling.
//!
//! The compositor's keyboard handler ([`crate::input`]) calls [`action_for`]
//! for every key press to decide whether it matches a binding. Matching presses
//! are intercepted (not forwarded to the focused client) and dispatched through
//! [`Oxmonad::handle_keybinding`].

use smithay::input::keyboard::{
    Keysym,
    ModifiersState,
};

use crate::{
    Oxmonad,
    config::{
        self,
        META,
    },
};

/// An action triggered by a keybinding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Spawn the configured [terminal emulator](config::TERMINAL).
    SpawnTerminal,
}

/// Resolves a key press to an [`Action`], if it matches a configured binding.
///
/// `modifiers` is the modifier state at the time of the press and `keysym` is
/// the resulting (already modifier-applied) symbol. Only the bindings hardcoded
/// here are recognised for now.
#[must_use]
pub fn action_for(modifiers: &ModifiersState, keysym: Keysym) -> Option<Action> {
    // META + Shift + Return -> launch the terminal.
    if META.is_active(modifiers) && modifiers.shift && keysym == Keysym::Return {
        return Some(Action::SpawnTerminal);
    }

    None
}

impl Oxmonad {
    /// Executes a keybinding [`Action`].
    #[expect(
        clippy::needless_pass_by_value,
        reason = "Dispatch hub for all keybindings; upcoming actions (quit, close window, focus) \
                  will carry owned payloads consumed here."
    )]
    pub fn handle_keybinding(&mut self, action: Action) {
        match action {
            Action::SpawnTerminal => spawn(config::TERMINAL),
        }
    }
}

/// Spawns `command` as a detached child process.
///
/// The child inherits `WAYLAND_DISPLAY` from our environment (set during
/// startup), so it connects back to this compositor.
fn spawn(command: &str) {
    if let Err(err) = std::process::Command::new(command).spawn() {
        tracing::warn!(%command, %err, "failed to spawn command");
    }
}
