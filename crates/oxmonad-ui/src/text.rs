//! Font loading, measuring and glyph rasterization.

use std::{
    borrow::Cow,
    cell::RefCell,
    collections::HashMap,
    fmt,
    rc::Rc,
};

use fontdue::{
    Font,
    FontSettings,
    Metrics,
};

/// Fira Mono, embedded so renders are identical on every machine.
const EMBEDDED_FONT: &[u8] = include_bytes!("../../../assets/FiraMono-Regular.ttf");
const EMBEDDED_FAMILY: &str = "Fira Mono";

/// Error returned when font data cannot be parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FontError(String);

impl fmt::Display for FontError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid font: {}", self.0)
    }
}

impl std::error::Error for FontError {}

/// A rasterized glyph: its placement metrics and 8-bit coverage bitmap.
#[derive(Debug)]
pub(crate) struct Glyph {
    pub metrics: Metrics,
    pub coverage: Vec<u8>,
}

/// The font every widget draws with, plus a cache of rasterized glyphs.
///
/// Sizes are always pixel sizes (the em size in device pixels). The cache is
/// keyed by character and size, so rendering the same widget repeatedly only
/// rasterizes each glyph once.
pub struct Fonts {
    font: Font,
    family: String,
    glyphs: RefCell<HashMap<(char, u32), Rc<Glyph>>>,
}

impl fmt::Debug for Fonts {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Fonts")
            .field("family", &self.family)
            .field("cached_glyphs", &self.glyphs.borrow().len())
            .finish_non_exhaustive()
    }
}

impl Default for Fonts {
    fn default() -> Self {
        Self::embedded()
    }
}

impl Fonts {
    /// The embedded Fira Mono.
    #[must_use]
    pub fn embedded() -> Self {
        Self::from_bytes(EMBEDDED_FONT, 0, EMBEDDED_FAMILY)
            .expect("the embedded Fira Mono is a valid font")
    }

    /// Looks `family` up among the installed fonts (fontconfig-style names
    /// such as `"Fira Mono"`, or the generic `monospace`, `sans-serif`,
    /// `serif`), falling back to the embedded Fira Mono when it is missing or
    /// unreadable. [`Fonts::family`] tells which one was loaded.
    #[must_use]
    pub fn from_family(family: &str) -> Self {
        Self::load_system(family).unwrap_or_else(Self::embedded)
    }

    /// Parses a TTF/OTF font (face `index` of a collection).
    pub fn from_bytes(data: &[u8], index: u32, family: &str) -> Result<Self, FontError> {
        let settings = FontSettings {
            collection_index: index,
            ..FontSettings::default()
        };
        let font = Font::from_bytes(data, settings).map_err(|e| FontError(e.to_owned()))?;
        Ok(Self {
            font,
            family: family.to_owned(),
            glyphs: RefCell::default(),
        })
    }

    fn load_system(family: &str) -> Option<Self> {
        let mut db = fontdb::Database::new();
        db.load_system_fonts();
        let wanted = match family.to_lowercase().as_str() {
            "monospace" => fontdb::Family::Monospace,
            "sans-serif" | "sans" => fontdb::Family::SansSerif,
            "serif" => fontdb::Family::Serif,
            _ => fontdb::Family::Name(family),
        };
        let id = db.query(&fontdb::Query {
            families: &[wanted],
            ..fontdb::Query::default()
        })?;
        let name = db
            .face(id)
            .and_then(|face| face.families.first())
            .map_or_else(|| family.to_owned(), |(name, _)| name.clone());
        db.with_face_data(id, |data, index| Self::from_bytes(data, index, &name).ok())
            .flatten()
    }

    /// The family name of the loaded font.
    #[must_use]
    pub fn family(&self) -> &str {
        &self.family
    }

    /// The width `text` occupies at `size_px`, in pixels.
    #[must_use]
    pub fn text_width(&self, text: &str, size_px: f32) -> f32 {
        text.chars()
            .map(|ch| self.glyph(ch, size_px).metrics.advance_width)
            .sum()
    }

    /// `text` shortened with an ellipsis so it fits in `max_width` pixels at
    /// `size_px`; unchanged when it already fits, empty when not even the
    /// ellipsis fits.
    #[must_use]
    pub fn ellipsize<'a>(&self, text: &'a str, size_px: f32, max_width: f32) -> Cow<'a, str> {
        if self.text_width(text, size_px) <= max_width {
            return Cow::Borrowed(text);
        }
        let ellipsis = if self.font.lookup_glyph_index('…') == 0 {
            "..."
        } else {
            "…"
        };
        let budget = max_width - self.text_width(ellipsis, size_px);
        if budget < 0.0 {
            return Cow::Borrowed("");
        }
        let mut width = 0.0;
        let mut end = 0;
        for (index, ch) in text.char_indices() {
            width += self.glyph(ch, size_px).metrics.advance_width;
            if width > budget {
                break;
            }
            end = index + ch.len_utf8();
        }
        Cow::Owned(format!("{}{ellipsis}", text[..end].trim_end()))
    }

    /// Ascent and descent (both positive, in pixels) at `size_px`.
    pub(crate) fn vertical_metrics(&self, size_px: f32) -> (f32, f32) {
        self.font
            .horizontal_line_metrics(size_px)
            .map_or((size_px * 0.8, size_px * 0.2), |line| {
                (line.ascent, -line.descent)
            })
    }

    /// The rasterized glyph for `ch` at `size_px`, from the cache if present.
    pub(crate) fn glyph(&self, ch: char, size_px: f32) -> Rc<Glyph> {
        let key = (ch, size_px.to_bits());
        Rc::clone(self.glyphs.borrow_mut().entry(key).or_insert_with(|| {
            let (metrics, coverage) = self.font.rasterize(ch, size_px);
            Rc::new(Glyph { metrics, coverage })
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn monospace_width_is_additive() {
        let fonts = Fonts::embedded();
        let one = fonts.text_width("a", 16.0);
        assert!(one > 0.0);
        assert!((fonts.text_width("abcd", 16.0) - 4.0 * one).abs() < 1e-3);
    }

    #[test]
    fn ellipsize_keeps_short_text_and_fits_long_text() {
        let fonts = Fonts::embedded();
        assert_eq!(fonts.ellipsize("short", 16.0, 1000.0), "short");
        let long = "a rather long window title that will not fit";
        let max = fonts.text_width("a rather long", 16.0);
        let shortened = fonts.ellipsize(long, 16.0, max);
        assert!(shortened.ends_with('…'), "{shortened:?}");
        assert!(fonts.text_width(&shortened, 16.0) <= max);
        assert_eq!(fonts.ellipsize(long, 16.0, 1.0), "");
    }

    #[test]
    fn unknown_family_falls_back_to_embedded() {
        let fonts = Fonts::from_family("No Such Font Family 1234");
        assert_eq!(fonts.family(), EMBEDDED_FAMILY);
    }
}
