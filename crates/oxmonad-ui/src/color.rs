//! Straight-alpha RGBA colors, parsed from `#rrggbb` / `#rrggbbaa`.

use std::{
    fmt,
    str::FromStr,
};

/// A straight (non-premultiplied) RGBA color with 8 bits per channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    /// An opaque color.
    #[must_use]
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    /// A color with explicit alpha.
    #[must_use]
    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    /// An opaque color from a `#rrggbb` literal, for constants.
    ///
    /// # Panics
    ///
    /// On malformed input (at compile time in a const context); use
    /// [`str::parse`] for runtime strings.
    #[must_use]
    pub const fn hex(literal: &str) -> Self {
        const fn nibble(byte: u8) -> u8 {
            match byte {
                b'0'..=b'9' => byte - b'0',
                b'a'..=b'f' => byte - b'a' + 10,
                b'A'..=b'F' => byte - b'A' + 10,
                _ => panic!("invalid hex digit in color literal"),
            }
        }
        let bytes = literal.as_bytes();
        assert!(
            bytes.len() == 7 && bytes[0] == b'#',
            "color literal must be #rrggbb"
        );
        Self::rgb(
            nibble(bytes[1]) << 4 | nibble(bytes[2]),
            nibble(bytes[3]) << 4 | nibble(bytes[4]),
            nibble(bytes[5]) << 4 | nibble(bytes[6]),
        )
    }

    /// The same color with its alpha replaced.
    #[must_use]
    pub const fn with_alpha(self, a: u8) -> Self {
        Self { a, ..self }
    }

    /// The premultiplied `[r, g, b, a]` channels.
    #[must_use]
    pub const fn premultiplied(self) -> [u8; 4] {
        [
            premultiply(self.r, self.a),
            premultiply(self.g, self.a),
            premultiply(self.b, self.a),
            self.a,
        ]
    }
}

/// `value * alpha / 255`, rounded to nearest.
#[expect(
    clippy::cast_possible_truncation,
    reason = "the quotient is at most 255"
)]
pub(crate) const fn premultiply(value: u8, alpha: u8) -> u8 {
    let product = value as u32 * alpha as u32 + 128;
    // (t + (t >> 8)) >> 8 with t = x + 128 is an exact rounded division by 255.
    ((product + (product >> 8)) >> 8) as u8
}

impl fmt::Display for Color {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{:02x}{:02x}{:02x}", self.r, self.g, self.b)?;
        if self.a != 255 {
            write!(f, "{:02x}", self.a)?;
        }
        Ok(())
    }
}

/// Error returned when a string is not `#rrggbb` or `#rrggbbaa`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseColorError(String);

impl fmt::Display for ParseColorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "invalid color {:?}: expected #rrggbb or #rrggbbaa",
            self.0
        )
    }
}

impl std::error::Error for ParseColorError {}

impl FromStr for Color {
    type Err = ParseColorError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let error = || ParseColorError(s.to_owned());
        let hex = s.strip_prefix('#').ok_or_else(error)?;
        if !matches!(hex.len(), 6 | 8) || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(error());
        }
        let channel = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).map_err(|_| error());
        let a = if hex.len() == 8 { channel(6)? } else { 255 };
        Ok(Self::rgba(channel(0)?, channel(2)?, channel(4)?, a))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_rgb_and_rgba() {
        assert_eq!("#F8E63A".parse(), Ok(Color::rgb(0xf8, 0xe6, 0x3a)));
        assert_eq!("#10203040".parse(), Ok(Color::rgba(0x10, 0x20, 0x30, 0x40)));
    }

    #[test]
    fn hex_literal_matches_parse() {
        assert_eq!(Ok(Color::hex("#1565C1")), "#1565c1".parse());
    }

    #[test]
    fn rejects_malformed_colors() {
        for bad in ["", "F8E63A", "#F8E63", "#F8E63AF", "#GGGGGG", "##00FF00"] {
            assert!(bad.parse::<Color>().is_err(), "{bad:?} should not parse");
        }
    }

    #[test]
    fn display_round_trips() {
        for text in ["#f8e63a", "#10203040"] {
            assert_eq!(
                text.parse::<Color>().map(|c| c.to_string()).as_deref(),
                Ok(text)
            );
        }
    }

    #[test]
    fn premultiply_rounds_exactly() {
        for value in 0..=255u8 {
            for alpha in 0..=255u8 {
                let exact = (f64::from(value) * f64::from(alpha) / 255.0).round();
                assert_eq!(f64::from(premultiply(value, alpha)), exact);
            }
        }
    }
}
