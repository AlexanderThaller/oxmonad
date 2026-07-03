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
    prompt::PromptKind,
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
    /// View the workspace at this index on the active monitor.
    ViewWorkspace(usize),
    /// Move the focused window to the workspace at this index.
    MoveToWorkspace(usize),
    /// Toggle back to the most recently hidden workspace.
    ToggleWorkspace,
    /// Swap the visible workspaces of the active monitor and its neighbour.
    SwapScreens,
    /// Focus the next monitor in physical order.
    FocusNextMonitor,
    /// Focus the previous monitor in physical order.
    FocusPrevMonitor,
    /// Move the focused window to the next monitor's workspace, without
    /// following it.
    ShiftToNextMonitor,
    /// Move the focused window to the previous monitor's workspace, without
    /// following it.
    ShiftToPrevMonitor,
    /// Retile every visible workspace (xmonad's refresh).
    Refresh,
    /// Spawn the configured [editor](config::EDITOR).
    SpawnEditor,
    /// Spawn the configured [screen locker](config::LOCKER).
    SpawnLocker,
    /// Toggle play/pause on the current media player.
    MediaPlayPause,
    /// Skip to the next media track.
    MediaNext,
    /// Skip to the previous media track.
    MediaPrev,
    /// Show or hide the built-in status bar (xmonad's toggle-struts).
    ToggleBar,
    /// Open a workspace prompt of the given kind.
    OpenPrompt(PromptKind),
    /// Remove the current workspace, merging its windows away.
    RemoveWorkspace,
    /// Advance the current workspace to the next layout in its ring.
    NextLayout,
    /// Reset the current workspace's layout ring to its defaults.
    ResetLayout,
    /// Increase the master count of the current layout.
    IncMaster,
    /// Decrease the master count of the current layout.
    DecMaster,
    /// Move the focused window one place down the stack (wrapping).
    SwapDown,
    /// Move the focused window one place up the stack (wrapping).
    SwapUp,
    /// Push the focused floating window back into the tiled layer.
    SinkWindow,
}

/// Resolves a key press to an [`Action`], if it matches a configured binding.
///
/// `modifiers` is the modifier state at the time of the press and `keysym` is
/// the *unshifted* (latin) symbol for the key — so a binding matches the same
/// physical key regardless of shift level or keyboard layout, like xmonad's
/// `(KeyMask, KeySym)` matching. Only the bindings hardcoded here are
/// recognised for now.
#[must_use]
pub fn action_for(modifiers: &ModifiersState, keysym: Keysym) -> Option<Action> {
    // Media keys work without any modifier, like the `(0, xK_XF86Audio…)`
    // bindings in the xmonad config (which also routes the mute key and
    // 0x1008ff18 to play/pause for the user's keyboard).
    match keysym {
        Keysym::XF86_AudioPlay | Keysym::XF86_AudioPause | Keysym::XF86_AudioMute => {
            return Some(Action::MediaPlayPause);
        }
        Keysym::XF86_AudioNext => return Some(Action::MediaNext),
        Keysym::XF86_AudioPrev => return Some(Action::MediaPrev),
        _ => {}
    }

    if !META.is_active(modifiers) {
        return None;
    }

    match (modifiers.shift, keysym) {
        // META + Shift + Return -> launch the terminal.
        (true, Keysym::Return) => Some(Action::SpawnTerminal),
        // META + Shift + c -> close the focused window.
        (true, Keysym::c) => Some(Action::CloseWindow),
        // META + Shift + q -> quit oxmonad.
        (true, Keysym::q) => Some(Action::Quit),
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
        // META + z -> swap workspaces between screens; with Shift -> toggle
        // back to the previous workspace.
        (false, Keysym::z) => Some(Action::SwapScreens),
        (true, Keysym::z) => Some(Action::ToggleWorkspace),
        // META + Left / Right -> focus the previous / next monitor; with
        // Shift -> send the focused window there instead.
        (false, Keysym::Right) => Some(Action::FocusNextMonitor),
        (false, Keysym::Left) => Some(Action::FocusPrevMonitor),
        (true, Keysym::Right) => Some(Action::ShiftToNextMonitor),
        (true, Keysym::Left) => Some(Action::ShiftToPrevMonitor),
        // META + Space -> next layout; with Shift -> reset the layout ring.
        (false, Keysym::space) => Some(Action::NextLayout),
        (true, Keysym::space) => Some(Action::ResetLayout),
        // META + t -> sink the focused floating window back into the tiling.
        (false, Keysym::t) => Some(Action::SinkWindow),
        // META + , / . -> increase / decrease the master window count.
        (false, Keysym::comma) => Some(Action::IncMaster),
        (false, Keysym::period) => Some(Action::DecMaster),
        // META + Shift + j / k -> swap the focused window down / up the stack.
        (true, Keysym::j) => Some(Action::SwapDown),
        (true, Keysym::k) => Some(Action::SwapUp),
        // META + n -> refresh the layout.
        (false, Keysym::n) => Some(Action::Refresh),
        // META + x -> launch the editor.
        (false, Keysym::x) => Some(Action::SpawnEditor),
        // META + Shift + l -> lock the screen.
        (true, Keysym::l) => Some(Action::SpawnLocker),
        // META + b -> toggle the built-in bar.
        (false, Keysym::b) => Some(Action::ToggleBar),
        // Dynamic workspaces: META + v select, META + m move, with Shift ->
        // copy; META + Shift + r rename, META + Shift + BackSpace remove.
        (false, Keysym::v) => Some(Action::OpenPrompt(PromptKind::Select)),
        (false, Keysym::m) => Some(Action::OpenPrompt(PromptKind::MoveTo)),
        (true, Keysym::m) => Some(Action::OpenPrompt(PromptKind::CopyTo)),
        (true, Keysym::r) => Some(Action::OpenPrompt(PromptKind::Rename)),
        (true, Keysym::BackSpace) => Some(Action::RemoveWorkspace),
        // META + 1..9 -> view that workspace; with Shift -> move window to it.
        (shift, _) => workspace_digit(keysym).map(|index| {
            if shift {
                Action::MoveToWorkspace(index)
            } else {
                Action::ViewWorkspace(index)
            }
        }),
    }
}

/// Maps a number-row keysym (`1`..`9`) to a zero-based workspace index.
fn workspace_digit(keysym: Keysym) -> Option<usize> {
    let digit = match keysym {
        Keysym::_1 => 1,
        Keysym::_2 => 2,
        Keysym::_3 => 3,
        Keysym::_4 => 4,
        Keysym::_5 => 5,
        Keysym::_6 => 6,
        Keysym::_7 => 7,
        Keysym::_8 => 8,
        Keysym::_9 => 9,
        _ => return None,
    };
    Some(digit - 1)
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
            Action::ViewWorkspace(index) => self.view_workspace(index),
            Action::MoveToWorkspace(index) => self.move_focused_to_workspace(index),
            Action::ToggleWorkspace => self.toggle_workspace(),
            Action::SwapScreens => self.swap_next_screen(),
            Action::FocusNextMonitor => self.focus_neighbour_monitor(1),
            Action::FocusPrevMonitor => self.focus_neighbour_monitor(-1),
            Action::ShiftToNextMonitor => self.shift_focused_to_neighbour_monitor(1),
            Action::ShiftToPrevMonitor => self.shift_focused_to_neighbour_monitor(-1),
            Action::Refresh => self.relayout(),
            Action::SpawnEditor => process::spawn(config::EDITOR),
            Action::SpawnLocker => process::spawn_shell(config::LOCKER, "locker"),
            Action::MediaPlayPause => process::spawn_shell(config::MEDIA_PLAY_PAUSE, "media"),
            Action::MediaNext => process::spawn_shell(config::MEDIA_NEXT, "media"),
            Action::MediaPrev => process::spawn_shell(config::MEDIA_PREV, "media"),
            Action::ToggleBar => self.toggle_bar(),
            Action::OpenPrompt(kind) => self.open_workspace_prompt(kind),
            Action::RemoveWorkspace => self.remove_current_workspace(),
            Action::NextLayout => self.next_layout(),
            Action::ResetLayout => self.reset_layout(),
            Action::SinkWindow => self.sink_focused_window(),
            Action::IncMaster => self.inc_master(),
            Action::DecMaster => self.dec_master(),
            Action::SwapDown => self.swap_down(),
            Action::SwapUp => self.swap_up(),
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
        // Matching is on the unshifted (base) keysym, so Shift + c arrives as
        // lowercase `c`, not `C`.
        assert_eq!(
            action_for(&meta_shift(), Keysym::c),
            Some(Action::CloseWindow)
        );
    }

    #[test]
    fn meta_shift_q_quits() {
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

    #[test]
    fn media_keys_work_without_modifiers() {
        let none = ModifiersState::default();
        assert_eq!(
            action_for(&none, Keysym::XF86_AudioPlay),
            Some(Action::MediaPlayPause)
        );
        assert_eq!(
            action_for(&none, Keysym::XF86_AudioNext),
            Some(Action::MediaNext)
        );
        assert_eq!(
            action_for(&none, Keysym::XF86_AudioPrev),
            Some(Action::MediaPrev)
        );
    }

    #[test]
    fn meta_z_swaps_screens_and_shift_toggles_workspace() {
        assert_eq!(action_for(&meta(), Keysym::z), Some(Action::SwapScreens));
        assert_eq!(
            action_for(&meta_shift(), Keysym::z),
            Some(Action::ToggleWorkspace)
        );
    }

    #[test]
    fn meta_arrows_navigate_monitors() {
        assert_eq!(
            action_for(&meta(), Keysym::Right),
            Some(Action::FocusNextMonitor)
        );
        assert_eq!(
            action_for(&meta(), Keysym::Left),
            Some(Action::FocusPrevMonitor)
        );
        assert_eq!(
            action_for(&meta_shift(), Keysym::Right),
            Some(Action::ShiftToNextMonitor)
        );
        assert_eq!(
            action_for(&meta_shift(), Keysym::Left),
            Some(Action::ShiftToPrevMonitor)
        );
    }

    #[test]
    fn spawn_and_bar_bindings() {
        assert_eq!(action_for(&meta(), Keysym::x), Some(Action::SpawnEditor));
        assert_eq!(
            action_for(&meta_shift(), Keysym::l),
            Some(Action::SpawnLocker)
        );
        assert_eq!(action_for(&meta(), Keysym::b), Some(Action::ToggleBar));
    }

    #[test]
    fn meta_space_cycles_and_resets_layout() {
        assert_eq!(action_for(&meta(), Keysym::space), Some(Action::NextLayout));
        assert_eq!(
            action_for(&meta_shift(), Keysym::space),
            Some(Action::ResetLayout)
        );
    }

    #[test]
    fn meta_comma_period_change_master_count() {
        assert_eq!(action_for(&meta(), Keysym::comma), Some(Action::IncMaster));
        assert_eq!(action_for(&meta(), Keysym::period), Some(Action::DecMaster));
    }

    #[test]
    fn meta_shift_jk_swap_window_in_stack() {
        assert_eq!(action_for(&meta_shift(), Keysym::j), Some(Action::SwapDown));
        assert_eq!(action_for(&meta_shift(), Keysym::k), Some(Action::SwapUp));
    }

    #[test]
    fn meta_digit_views_workspace() {
        assert_eq!(
            action_for(&meta(), Keysym::_1),
            Some(Action::ViewWorkspace(0))
        );
        assert_eq!(
            action_for(&meta(), Keysym::_9),
            Some(Action::ViewWorkspace(8))
        );
    }

    #[test]
    fn meta_shift_digit_moves_window_to_workspace() {
        assert_eq!(
            action_for(&meta_shift(), Keysym::_1),
            Some(Action::MoveToWorkspace(0))
        );
        assert_eq!(
            action_for(&meta_shift(), Keysym::_4),
            Some(Action::MoveToWorkspace(3))
        );
    }
}
