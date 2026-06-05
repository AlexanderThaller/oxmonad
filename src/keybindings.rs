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
    process,
};

/// An action triggered by a keybinding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Spawn the configured [terminal emulator](config::TERMINAL).
    SpawnTerminal,
    /// Spawn the configured [application launcher](config::LAUNCHER).
    SpawnLauncher,
    /// Swap the focused window with the master window.
    SwapMaster,
    /// Focus the next window in tiling order.
    FocusNext,
    /// Focus the previous window in tiling order.
    FocusPrev,
    /// Ask the focused window to close.
    CloseWindow,
    /// Quit the compositor.
    Quit,
    /// Expand the focused window vertically within its layout column.
    ExpandFocused,
    /// Shrink the focused window vertically within its layout column.
    ShrinkFocused,
    /// Make the master area narrower.
    ShrinkMaster,
    /// Make the master area wider.
    ExpandMaster,
    /// Move the focused window to the next monitor.
    MoveToOtherMonitor,
}

/// Resolves a key press to an [`Action`], if it matches a configured binding.
///
/// `modifiers` is the modifier state at the time of the press and `keysym` is
/// the resulting (already modifier-applied) symbol. Only the bindings hardcoded
/// here are recognised for now.
#[must_use]
pub fn action_for(modifiers: &ModifiersState, keysym: Keysym) -> Option<Action> {
    if !META.is_active(modifiers) {
        return None;
    }

    match (modifiers.shift, keysym) {
        // META + Shift + Return -> launch the terminal.
        (true, Keysym::Return) => Some(Action::SpawnTerminal),
        // META + Shift + c -> close the focused window.
        (true, Keysym::C | Keysym::c) => Some(Action::CloseWindow),
        // META + Shift + q -> quit oxmonad.
        (true, Keysym::Q | Keysym::q) => Some(Action::Quit),
        // META + Return -> promote the focused window to master.
        (false, Keysym::Return) => Some(Action::SwapMaster),
        // META + p -> open the application launcher.
        (false, Keysym::p) => Some(Action::SpawnLauncher),
        // META + o -> move the focused window to the next monitor.
        (false, Keysym::o) => Some(Action::MoveToOtherMonitor),
        // META + j / k -> move focus down / up the window stack.
        (false, Keysym::j) => Some(Action::FocusNext),
        (false, Keysym::k) => Some(Action::FocusPrev),
        // META + h / l -> shrink / expand the master area (xmonad defaults).
        (false, Keysym::h) => Some(Action::ShrinkMaster),
        (false, Keysym::l) => Some(Action::ExpandMaster),
        // META + Up / Down -> resize the focused tile vertically.
        (false, Keysym::Up) => Some(Action::ExpandFocused),
        (false, Keysym::Down) => Some(Action::ShrinkFocused),
        _ => None,
    }
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
            Action::SpawnTerminal => process::spawn(config::TERMINAL),
            Action::SpawnLauncher => process::spawn_shell(config::LAUNCHER, "launcher"),
            Action::SwapMaster => self.swap_master(),
            Action::FocusNext => self.focus_next(),
            Action::FocusPrev => self.focus_prev(),
            Action::CloseWindow => self.close_focused_window(),
            Action::Quit => self.loop_signal.stop(),
            Action::ExpandFocused => self.expand_focused_window(),
            Action::ShrinkFocused => self.shrink_focused_window(),
            Action::ShrinkMaster => self.shrink_master_area(),
            Action::ExpandMaster => self.expand_master_area(),
            Action::MoveToOtherMonitor => self.move_focused_to_next_monitor(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta_shift() -> ModifiersState {
        ModifiersState {
            logo: true,
            shift: true,
            ..ModifiersState::default()
        }
    }

    fn meta() -> ModifiersState {
        ModifiersState {
            logo: true,
            ..ModifiersState::default()
        }
    }

    #[test]
    fn meta_shift_c_closes_window() {
        assert_eq!(
            action_for(&meta_shift(), Keysym::C),
            Some(Action::CloseWindow)
        );
        assert_eq!(
            action_for(&meta_shift(), Keysym::c),
            Some(Action::CloseWindow)
        );
    }

    #[test]
    fn meta_shift_q_quits() {
        assert_eq!(action_for(&meta_shift(), Keysym::Q), Some(Action::Quit));
        assert_eq!(action_for(&meta_shift(), Keysym::q), Some(Action::Quit));
    }

    #[test]
    fn meta_up_down_resize_focused_tile() {
        assert_eq!(action_for(&meta(), Keysym::Up), Some(Action::ExpandFocused));
        assert_eq!(
            action_for(&meta(), Keysym::Down),
            Some(Action::ShrinkFocused)
        );
    }

    #[test]
    fn meta_h_l_resize_master_area() {
        assert_eq!(action_for(&meta(), Keysym::h), Some(Action::ShrinkMaster));
        assert_eq!(action_for(&meta(), Keysym::l), Some(Action::ExpandMaster));
    }
}
