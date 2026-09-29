//! Key and pointer combinations as written in the config.
//!
//! Key names are kept verbatim (xkb keysym names such as `Return`, `j`,
//! `comma` or `XF86AudioPlay`); resolving them to keysyms is the binary's
//! job.

use std::{
    fmt,
    ops::BitOr,
};

/// A set of modifier keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Modifiers(pub u8);

impl Modifiers {
    /// No modifier.
    pub const EMPTY: Self = Self(0);
    /// The logo/Super key (`Mod4`).
    pub const SUPER: Self = Self(1);
    /// Alt (`Mod1`).
    pub const ALT: Self = Self(1 << 1);
    /// Control.
    pub const CTRL: Self = Self(1 << 2);
    /// Shift.
    pub const SHIFT: Self = Self(1 << 3);

    /// Display order and names.
    const NAMES: [(Self, &'static str); 4] = [
        (Self::SUPER, "Super"),
        (Self::CTRL, "Ctrl"),
        (Self::ALT, "Alt"),
        (Self::SHIFT, "Shift"),
    ];

    /// Whether every modifier in `other` is set.
    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// Whether no modifier is set.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Parses the `modifier` config value: one or more `+`-separated
    /// modifier names (`Mod` itself is not allowed).
    pub fn parse(s: &str) -> Result<Self, KeyError> {
        let mut mods = Self::EMPTY;
        for token in s.split('+') {
            let token = token.trim();
            if token.to_lowercase() == "mod" {
                return Err(KeyError::ModInModifier(s.to_owned()));
            }
            mods = mods
                | Self::from_name(token)
                    .ok_or_else(|| KeyError::UnknownModifier(token.to_owned()))?;
        }
        if mods.is_empty() {
            return Err(KeyError::Empty(s.to_owned()));
        }
        Ok(mods)
    }

    /// Parses a single modifier name, case-insensitively.
    fn from_name(name: &str) -> Option<Self> {
        match name.to_lowercase().as_str() {
            "super" | "logo" | "mod4" | "win" => Some(Self::SUPER),
            "alt" | "mod1" => Some(Self::ALT),
            "ctrl" | "control" => Some(Self::CTRL),
            "shift" => Some(Self::SHIFT),
            _ => None,
        }
    }

    /// Parses `Mod+Shift+…`-style prefixes: every token but the last is a
    /// modifier (`Mod` resolves to `modifier`); returns the last token.
    fn split_combo(s: &str, modifier: Self) -> Result<(Self, &str), KeyError> {
        let mut tokens: Vec<&str> = s.split('+').map(str::trim).collect();
        let last = tokens.pop().unwrap_or_default();
        if last.is_empty() {
            return Err(KeyError::Empty(s.to_owned()));
        }
        let mut mods = Self::EMPTY;
        for token in tokens {
            mods = mods
                | if token.to_lowercase() == "mod" {
                    modifier
                } else {
                    Self::from_name(token)
                        .ok_or_else(|| KeyError::UnknownModifier(token.to_owned()))?
                };
        }
        Ok((mods, last))
    }
}

impl BitOr for Modifiers {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

impl fmt::Display for Modifiers {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut first = true;
        for (flag, name) in Self::NAMES {
            if self.contains(flag) {
                if !first {
                    f.write_str("+")?;
                }
                f.write_str(name)?;
                first = false;
            }
        }
        Ok(())
    }
}

/// Error for a malformed key/pointer combination or modifier.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum KeyError {
    /// The combination has no key/button (e.g. `""` or `"Mod+"`).
    #[error("`{0}` has no key")]
    Empty(String),
    /// A modifier token is not known.
    #[error("unknown modifier `{0}`")]
    UnknownModifier(String),
    /// `Mod` was used in the `modifier` setting itself.
    #[error("`{0}`: `Mod` cannot be used to define the modifier")]
    ModInModifier(String),
    /// A pointer button name is not known.
    #[error("unknown pointer button `{0}`")]
    UnknownButton(String),
}

/// A keyboard shortcut: modifiers plus an unresolved keysym name.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct KeyCombo {
    /// Held modifiers.
    pub mods: Modifiers,
    /// The keysym name, verbatim (e.g. `Return`, `j`, `XF86AudioPlay`).
    pub key: String,
}

impl KeyCombo {
    /// Parses `Mod+Shift+Return`; `Mod` resolves to `modifier`.
    pub fn parse(s: &str, modifier: Modifiers) -> Result<Self, KeyError> {
        let (mods, key) = Modifiers::split_combo(s, modifier)?;
        Ok(Self {
            mods,
            key: key.to_owned(),
        })
    }
}

impl fmt::Display for KeyCombo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.mods.is_empty() {
            f.write_str(&self.key)
        } else {
            write!(f, "{}+{}", self.mods, self.key)
        }
    }
}

/// A pointer button.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PointerButton {
    /// Left button (`Button1`).
    Left,
    /// Right button (`Button3`).
    Right,
    /// Middle button (`Button2`).
    Middle,
    /// Any other button, by Linux evdev code.
    Other(u32),
}

impl PointerButton {
    /// The Linux evdev button code (`BTN_LEFT` = `0x110`, …).
    #[must_use]
    pub const fn evdev_code(self) -> u32 {
        match self {
            Self::Left => 0x110,
            Self::Right => 0x111,
            Self::Middle => 0x112,
            Self::Other(code) => code,
        }
    }

    /// Parses `Button1`/`Button2`/`Button3` (X11 numbering: left, middle,
    /// right), `Button8`/`Button9` (side/extra), or `Left`/`Middle`/`Right`
    /// (case-insensitive).
    pub fn parse(s: &str) -> Result<Self, KeyError> {
        match s.to_lowercase().as_str() {
            "button1" | "left" => Ok(Self::Left),
            "button2" | "middle" => Ok(Self::Middle),
            "button3" | "right" => Ok(Self::Right),
            "button8" | "side" => Ok(Self::Other(0x113)),
            "button9" | "extra" => Ok(Self::Other(0x114)),
            _ => Err(KeyError::UnknownButton(s.to_owned())),
        }
    }
}

impl fmt::Display for PointerButton {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Left => f.write_str("Button1"),
            Self::Middle => f.write_str("Button2"),
            Self::Right => f.write_str("Button3"),
            Self::Other(0x113) => f.write_str("Button8"),
            Self::Other(0x114) => f.write_str("Button9"),
            Self::Other(code) => write!(f, "Button(0x{code:x})"),
        }
    }
}

/// A pointer shortcut: modifiers plus a button.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PointerCombo {
    /// Held modifiers.
    pub mods: Modifiers,
    /// The pressed button.
    pub button: PointerButton,
}

impl PointerCombo {
    /// Parses `Mod+Button1`; `Mod` resolves to `modifier`.
    pub fn parse(s: &str, modifier: Modifiers) -> Result<Self, KeyError> {
        let (mods, button) = Modifiers::split_combo(s, modifier)?;
        Ok(Self {
            mods,
            button: PointerButton::parse(button)?,
        })
    }
}

impl fmt::Display for PointerCombo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.mods.is_empty() {
            write!(f, "{}", self.button)
        } else {
            write!(f, "{}+{}", self.mods, self.button)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modifier_setting() {
        assert_eq!(Modifiers::parse("Super"), Ok(Modifiers::SUPER));
        assert_eq!(Modifiers::parse("alt"), Ok(Modifiers::ALT));
        assert_eq!(
            Modifiers::parse("Ctrl+Alt"),
            Ok(Modifiers::CTRL | Modifiers::ALT)
        );
        assert_eq!(Modifiers::parse("Mod4"), Ok(Modifiers::SUPER));
        assert!(matches!(
            Modifiers::parse("Mod"),
            Err(KeyError::ModInModifier(_))
        ));
        assert!(matches!(
            Modifiers::parse("Hyper"),
            Err(KeyError::UnknownModifier(_))
        ));
        assert!(Modifiers::parse("").is_err());
    }

    #[test]
    fn key_combos() {
        let combo = KeyCombo::parse("Mod+Shift+Return", Modifiers::SUPER).expect("valid");
        assert_eq!(combo.mods, Modifiers::SUPER | Modifiers::SHIFT);
        assert_eq!(combo.key, "Return");
        assert_eq!(combo.to_string(), "Super+Shift+Return");

        let combo = KeyCombo::parse("mod+j", Modifiers::ALT).expect("valid");
        assert_eq!(combo.mods, Modifiers::ALT);
        assert_eq!(combo.key, "j");

        let combo = KeyCombo::parse("XF86AudioPlay", Modifiers::SUPER).expect("valid");
        assert!(combo.mods.is_empty());
        assert_eq!(combo.to_string(), "XF86AudioPlay");

        // Aliases collide once resolved.
        assert_eq!(
            KeyCombo::parse("Super+j", Modifiers::SUPER),
            KeyCombo::parse("Mod+j", Modifiers::SUPER)
        );
        assert_eq!(
            KeyCombo::parse("Control+Mod1+x", Modifiers::SUPER),
            KeyCombo::parse("Alt+Ctrl+x", Modifiers::SUPER)
        );
    }

    #[test]
    fn key_combo_errors() {
        assert!(matches!(
            KeyCombo::parse("", Modifiers::SUPER),
            Err(KeyError::Empty(_))
        ));
        assert!(matches!(
            KeyCombo::parse("Mod+", Modifiers::SUPER),
            Err(KeyError::Empty(_))
        ));
        assert!(matches!(
            KeyCombo::parse("Hyper+j", Modifiers::SUPER),
            Err(KeyError::UnknownModifier(_))
        ));
    }

    #[test]
    fn modifier_contains() {
        let mods = Modifiers::SUPER | Modifiers::SHIFT;
        assert!(mods.contains(Modifiers::SUPER));
        assert!(mods.contains(Modifiers::EMPTY));
        assert!(!mods.contains(Modifiers::ALT));
        assert!(Modifiers::EMPTY.is_empty());
        assert_eq!(Modifiers::EMPTY.to_string(), "");
    }

    #[test]
    fn pointer_combos() {
        let combo = PointerCombo::parse("Mod+Button1", Modifiers::SUPER).expect("valid");
        assert_eq!(combo.button, PointerButton::Left);
        assert_eq!(combo.button.evdev_code(), 0x110);
        assert_eq!(combo.to_string(), "Super+Button1");
        let combo = PointerCombo::parse("Alt+right", Modifiers::SUPER).expect("valid");
        assert_eq!(combo.mods, Modifiers::ALT);
        assert_eq!(combo.button.evdev_code(), 0x111);
        assert_eq!(PointerButton::parse("Button2"), Ok(PointerButton::Middle));
        assert_eq!(PointerButton::Middle.evdev_code(), 0x112);
        assert_eq!(
            PointerButton::parse("Button8"),
            Ok(PointerButton::Other(0x113))
        );
        assert!(matches!(
            PointerCombo::parse("Mod+Button7", Modifiers::SUPER),
            Err(KeyError::UnknownButton(_))
        ));
    }
}
