//! Compile-time configuration for oxmonad.
//!
//! Keybindings and a few program choices are hardcoded here for now. This is
//! intended to be replaced by a proper runtime/user configuration mechanism
//! later, but keeping it centralised makes the eventual migration easy.

use smithay::input::keyboard::ModifiersState;

/// The primary modifier used by every oxmonad keybinding — xmonad's "mod key".
///
/// Defaults to [`Modifier::Super`] (the "logo"/Windows key). Change this single
/// constant to rebind every shortcut to a different modifier.
pub const META: Modifier = Modifier::Super;

/// Terminal emulator launched by the spawn-terminal keybinding.
pub const TERMINAL: &str = "alacritty";

/// A logical modifier key that a keybinding can require.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[expect(
    dead_code,
    reason = "`Alt`/`Control` are selectable values for `META`; only the currently chosen variant \
              is constructed."
)]
pub enum Modifier {
    /// The "Super"/"logo"/Windows key.
    Super,
    /// The Alt key.
    Alt,
    /// The Control key.
    Control,
}

impl Modifier {
    /// Returns whether this modifier is currently held in `modifiers`.
    #[must_use]
    pub fn is_active(self, modifiers: &ModifiersState) -> bool {
        match self {
            Self::Super => modifiers.logo,
            Self::Alt => modifiers.alt,
            Self::Control => modifiers.ctrl,
        }
    }
}
