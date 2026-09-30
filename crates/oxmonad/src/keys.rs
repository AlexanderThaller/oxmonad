//! Resolving config key combinations to xkb keysyms and river modifier
//! masks, and translating keys typed into the prompt, layout picker or help
//! overlay.

use kbvm::Keysym;
use oxmonad_core::{
    config::{
        KeyCombo,
        Modifiers,
    },
    prompt::PromptKey,
};

/// `river_seat_v1.modifiers` bits.
pub mod river_mods {
    /// Shift.
    pub const SHIFT: u32 = 1;
    /// Control.
    pub const CTRL: u32 = 4;
    /// Mod1, commonly Alt.
    pub const MOD1: u32 = 8;
    /// Mod4, commonly Super/Logo.
    pub const MOD4: u32 = 64;
}

/// The `river_seat_v1.modifiers` bitmask for `mods`.
#[must_use]
pub fn river_modifiers(mods: Modifiers) -> u32 {
    [
        (Modifiers::SHIFT, river_mods::SHIFT),
        (Modifiers::CTRL, river_mods::CTRL),
        (Modifiers::ALT, river_mods::MOD1),
        (Modifiers::SUPER, river_mods::MOD4),
    ]
    .into_iter()
    .filter(|(flag, _)| mods.contains(*flag))
    .fold(0, |mask, (_, bit)| mask | bit)
}

/// The keysym a key name stands for: an exact xkb keysym name (`Return`,
/// `comma`, `XF86AudioPlay`), else a case-insensitive match (`return`).
/// Letters resolve to their lowercase keysym, because river matches
/// bindings against the unshifted keysym plus the held modifiers
/// (`Mod+Shift+J` means the `j` key with Shift).
#[must_use]
pub fn keysym(name: &str) -> Option<Keysym> {
    let sym = Keysym::from_str(name).or_else(|| Keysym::from_str_insensitive(name))?;
    Some(sym.to_lowercase())
}

/// A binding river can match: a keysym plus a modifier mask.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ResolvedKey {
    /// The xkb keysym.
    pub keysym: u32,
    /// `river_seat_v1.modifiers`.
    pub modifiers: u32,
}

/// Resolves a config key combination.
#[must_use]
pub fn resolve(combo: &KeyCombo) -> Option<ResolvedKey> {
    Some(ResolvedKey {
        keysym: keysym(&combo.key)?.0,
        modifiers: river_modifiers(combo.mods),
    })
}

/// Translates a key typed into the prompt (keysym plus whether Control and
/// Shift are held) into a [`PromptKey`], like `XMonad.Prompt`'s default
/// key map.
#[must_use]
pub fn prompt_key(sym: Keysym, ctrl: bool, shift: bool) -> Option<PromptKey> {
    use kbvm::syms;

    if ctrl {
        return match sym.to_lowercase() {
            syms::w => Some(PromptKey::DeleteWord),
            syms::k => Some(PromptKey::KillToEnd),
            syms::u => Some(PromptKey::KillToStart),
            syms::a => Some(PromptKey::Home),
            syms::e => Some(PromptKey::End),
            syms::b => Some(PromptKey::Left),
            syms::f => Some(PromptKey::Right),
            syms::d => Some(PromptKey::Delete),
            syms::h => Some(PromptKey::Backspace),
            syms::g | syms::bracketleft | syms::c => Some(PromptKey::Escape),
            syms::m | syms::j => Some(PromptKey::Return),
            _ => None,
        };
    }
    Some(match sym {
        syms::Return | syms::KP_Enter => PromptKey::Return,
        syms::Escape => PromptKey::Escape,
        syms::BackSpace => PromptKey::Backspace,
        syms::Delete | syms::KP_Delete => PromptKey::Delete,
        syms::Tab if shift => PromptKey::BackTab,
        syms::Tab => PromptKey::Tab,
        syms::ISO_Left_Tab => PromptKey::BackTab,
        syms::Left | syms::KP_Left => PromptKey::Left,
        syms::Right | syms::KP_Right => PromptKey::Right,
        syms::Up | syms::KP_Up => PromptKey::Up,
        syms::Down | syms::KP_Down => PromptKey::Down,
        syms::Page_Up | syms::KP_Page_Up => PromptKey::PageUp,
        syms::Page_Down | syms::KP_Page_Down => PromptKey::PageDown,
        syms::Home | syms::KP_Home => PromptKey::Home,
        syms::End | syms::KP_End => PromptKey::End,
        _ => PromptKey::Char(sym.char().filter(|c| !c.is_control())?),
    })
}

#[cfg(test)]
mod tests {
    use oxmonad_core::config::Config;

    use super::*;

    #[test]
    fn every_default_binding_resolves() {
        let config = Config::defaults();
        assert_ne!(config.keybindings.len(), 0);
        for binding in &config.keybindings {
            assert!(
                resolve(&binding.combo).is_some(),
                "{} does not resolve",
                binding.combo
            );
        }
    }

    #[test]
    fn names() {
        let sym = |name| keysym(name).map(|s| s.0);
        assert_eq!(sym("Return"), Some(0xff0d));
        assert_eq!(sym("return"), Some(0xff0d));
        assert_eq!(sym("BackSpace"), Some(0xff08));
        assert_eq!(sym("XF86AudioPlay"), Some(0x1008_ff14));
        assert_eq!(sym("comma"), Some(u32::from(b',')));
        assert_eq!(sym("period"), Some(u32::from(b'.')));
        assert_eq!(sym("space"), Some(u32::from(b' ')));
        assert_eq!(sym("grave"), Some(u32::from(b'`')));
        assert_eq!(sym("j"), Some(u32::from(b'j')));
        assert_eq!(sym("J"), Some(u32::from(b'j')));
        assert_eq!(sym("1"), Some(u32::from(b'1')));
        assert_eq!(sym("Up"), Some(0xff52));
        assert_eq!(sym("NoSuchKey"), None);
    }

    #[test]
    fn help_binding_resolves_to_shift_slash() {
        // river matches the unshifted keysym plus the modifiers.
        let config = Config::defaults();
        let help = config
            .keybindings
            .iter()
            .find(|b| b.action == oxmonad_core::Action::Help)
            .expect("help is bound");
        assert_eq!(
            resolve(&help.combo),
            Some(ResolvedKey {
                keysym: u32::from(b'/'),
                modifiers: river_mods::MOD4 | river_mods::SHIFT,
            })
        );
    }

    #[test]
    fn modifiers() {
        let combo = KeyCombo::parse("Mod+Shift+Return", Modifiers::SUPER).expect("valid");
        assert_eq!(
            resolve(&combo),
            Some(ResolvedKey {
                keysym: 0xff0d,
                modifiers: river_mods::MOD4 | river_mods::SHIFT,
            })
        );
        let combo = KeyCombo::parse("Ctrl+Alt+x", Modifiers::SUPER).expect("valid");
        assert_eq!(
            resolve(&combo).map(|k| k.modifiers),
            Some(river_mods::CTRL | river_mods::MOD1)
        );
        let combo = KeyCombo::parse("XF86AudioNext", Modifiers::SUPER).expect("valid");
        assert_eq!(resolve(&combo).map(|k| k.modifiers), Some(0));
    }

    #[test]
    fn prompt_keys() {
        use kbvm::syms;
        assert_eq!(
            prompt_key(syms::a, false, false),
            Some(PromptKey::Char('a'))
        );
        assert_eq!(prompt_key(syms::A, false, true), Some(PromptKey::Char('A')));
        assert_eq!(
            prompt_key(syms::Return, false, false),
            Some(PromptKey::Return)
        );
        assert_eq!(prompt_key(syms::Tab, false, true), Some(PromptKey::BackTab));
        assert_eq!(
            prompt_key(syms::ISO_Left_Tab, false, true),
            Some(PromptKey::BackTab)
        );
        assert_eq!(
            prompt_key(syms::w, true, false),
            Some(PromptKey::DeleteWord)
        );
        assert_eq!(prompt_key(syms::Down, false, false), Some(PromptKey::Down));
        assert_eq!(prompt_key(syms::KP_Up, false, false), Some(PromptKey::Up));
        assert_eq!(
            prompt_key(syms::Page_Down, false, false),
            Some(PromptKey::PageDown)
        );
        assert_eq!(
            prompt_key(syms::KP_Page_Up, false, false),
            Some(PromptKey::PageUp)
        );
        assert_eq!(
            prompt_key(syms::j, false, false),
            Some(PromptKey::Char('j'))
        );
        assert_eq!(prompt_key(syms::Shift_L, false, false), None);
        assert_eq!(prompt_key(syms::F1, false, false), None);
    }
}
