//! `#RRGGBB[AA]` colors.

use std::fmt;

use serde::{
    Deserialize,
    Deserializer,
    de,
};

/// A straight-alpha sRGB color.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Color {
    /// Red.
    pub r: u8,
    /// Green.
    pub g: u8,
    /// Blue.
    pub b: u8,
    /// Alpha (`255` is opaque).
    pub a: u8,
}

/// Error for a malformed color string.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid color `{0}`, expected `#RRGGBB` or `#RRGGBBAA`")]
pub struct ColorError(pub String);

impl Color {
    /// Creates an opaque color.
    #[must_use]
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    /// Parses `#RRGGBB` or `#RRGGBBAA` (hex digits in either case).
    pub fn parse(s: &str) -> Result<Self, ColorError> {
        let err = || ColorError(s.to_owned());
        let hex = s.strip_prefix('#').ok_or_else(err)?;
        if !(hex.len() == 6 || hex.len() == 8) || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(err());
        }
        let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).map_err(|_| err());
        Ok(Self {
            r: byte(0)?,
            g: byte(2)?,
            b: byte(4)?,
            a: if hex.len() == 8 { byte(6)? } else { 255 },
        })
    }

    /// The color as `0xAARRGGBB` (straight alpha).
    #[must_use]
    pub const fn to_argb_u32(self) -> u32 {
        u32::from_be_bytes([self.a, self.r, self.g, self.b])
    }

    /// The components as `[r, g, b, a]`.
    #[must_use]
    pub const fn to_rgba(self) -> [u8; 4] {
        [self.r, self.g, self.b, self.a]
    }
}

impl fmt::Display for Color {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{:02X}{:02X}{:02X}", self.r, self.g, self.b)?;
        if self.a != 255 {
            write!(f, "{:02X}", self.a)?;
        }
        Ok(())
    }
}

impl<'de> Deserialize<'de> for Color {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Self::parse(&s).map_err(de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_rgb_and_rgba() {
        assert_eq!(Color::parse("#C62828"), Ok(Color::rgb(0xC6, 0x28, 0x28)));
        assert_eq!(
            Color::parse("#1565c180"),
            Ok(Color {
                r: 0x15,
                g: 0x65,
                b: 0xC1,
                a: 0x80
            })
        );
    }

    #[test]
    fn rejects_malformed() {
        for bad in [
            "C62828",
            "#C6282",
            "#C628281",
            "#GG0000",
            "",
            "#",
            "#ÄÄÄÄÄÄ",
        ] {
            assert!(Color::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn conversions() {
        let c = Color::parse("#11223344").expect("valid");
        assert_eq!(c.to_argb_u32(), 0x4411_2233);
        assert_eq!(c.to_rgba(), [0x11, 0x22, 0x33, 0x44]);
        assert_eq!(c.to_string(), "#11223344");
        assert_eq!(Color::rgb(1, 2, 3).to_string(), "#010203");
    }
}
