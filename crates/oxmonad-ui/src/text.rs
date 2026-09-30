//! Font loading, measuring and glyph rasterization.
//!
//! Outlines are hinted by `skrifa`'s automatic hinter in its normal (full)
//! mode, which snaps stems to whole pixels horizontally and vertically, and
//! rasterized by `zeno`. Advances are rounded to whole pixels, so every
//! glyph starts on a pixel boundary and keeps its hinted stems crisp.

use std::{
    borrow::Cow,
    cell::RefCell,
    collections::HashMap,
    fmt,
    rc::Rc,
};

use skrifa::{
    FontRef,
    GlyphId,
    MetadataProvider,
    instance::{
        LocationRef,
        Size,
    },
    outline::{
        DrawSettings,
        Engine,
        HintingInstance,
        HintingOptions,
        OutlinePen,
        SmoothMode,
        Target,
    },
};
use zeno::{
    Command,
    Mask,
    PathBuilder,
    Transform,
};

/// Fira Mono, embedded so renders are identical on every machine.
const EMBEDDED_FONT: &[u8] = include_bytes!("../../../assets/FiraMono-Regular.ttf");
const EMBEDDED_FAMILY: &str = "Fira Mono";

/// `FreeType`'s default LCD filter: spreads each subpixel's coverage over its
/// neighbours to tame color fringes. The weights sum to 256.
const LCD_FILTER: [u32; 5] = [8, 77, 86, 77, 8];

/// Error returned when font data cannot be parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FontError(String);

impl fmt::Display for FontError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid font: {}", self.0)
    }
}

impl std::error::Error for FontError {}

/// How glyph edges are antialiased.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum Antialias {
    /// One coverage value per pixel.
    #[default]
    Grayscale,
    /// Subpixel antialiasing for screens whose subpixels are ordered red,
    /// green, blue from left to right. Only used on opaque backgrounds.
    Rgb,
    /// Subpixel antialiasing for blue, green, red subpixel order.
    Bgr,
}

/// A rasterized glyph: its placement relative to the pen position on the
/// baseline, its advance and its coverage bitmap.
#[derive(Debug)]
pub(crate) struct Glyph {
    /// Offset from the pen position to the bitmap's left edge.
    pub left: i32,
    /// Offset from the baseline to the bitmap's top edge (negative is up).
    pub top: i32,
    pub width: usize,
    pub height: usize,
    /// Whole pixels the pen moves on.
    pub advance: f32,
    /// Coverage row by row: one byte per pixel for [`Antialias::Grayscale`],
    /// otherwise three (red, green, blue) per pixel.
    pub coverage: Vec<u8>,
}

/// Collects outline commands, flipping y so it points down.
#[derive(Default)]
struct Pen(Vec<Command>);

impl OutlinePen for Pen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.move_to([x, -y]);
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.0.line_to([x, -y]);
    }

    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        self.0.quad_to([cx0, -cy0], [x, -y]);
    }

    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        self.0.curve_to([cx0, -cy0], [cx1, -cy1], [x, -y]);
    }

    fn close(&mut self) {
        self.0.close();
    }
}

/// The font every widget draws with, plus a cache of rasterized glyphs.
///
/// Sizes are always pixel sizes (the em size in device pixels). The cache is
/// keyed by character, size and antialiasing mode, so rendering the same
/// widget repeatedly only rasterizes each glyph once.
pub struct Fonts {
    data: Cow<'static, [u8]>,
    index: u32,
    family: String,
    /// Hinting instances by size; `None` when the font cannot be hinted.
    hinters: RefCell<HashMap<u32, Option<HintingInstance>>>,
    glyphs: RefCell<HashMap<(char, u32, Antialias), Rc<Glyph>>>,
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
        Self::new(Cow::Borrowed(EMBEDDED_FONT), 0, EMBEDDED_FAMILY)
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
        Self::new(Cow::Owned(data.to_vec()), index, family)
    }

    fn new(data: Cow<'static, [u8]>, index: u32, family: &str) -> Result<Self, FontError> {
        let font = FontRef::from_index(&data, index).map_err(|e| FontError(e.to_string()))?;
        if font.outline_glyphs().format().is_none() {
            return Err(FontError("no scalable outlines".to_owned()));
        }
        Ok(Self {
            data,
            index,
            family: family.to_owned(),
            hinters: RefCell::default(),
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

    fn font(&self) -> FontRef<'_> {
        FontRef::from_index(&self.data, self.index).expect("the font was validated on load")
    }

    /// The width `text` occupies at `size_px`, in pixels.
    #[must_use]
    pub fn text_width(&self, text: &str, size_px: f32) -> f32 {
        text.chars().map(|ch| self.advance(ch, size_px)).sum()
    }

    /// `text` shortened with an ellipsis so it fits in `max_width` pixels at
    /// `size_px`; unchanged when it already fits, empty when not even the
    /// ellipsis fits.
    #[must_use]
    pub fn ellipsize<'a>(&self, text: &'a str, size_px: f32, max_width: f32) -> Cow<'a, str> {
        if self.text_width(text, size_px) <= max_width {
            return Cow::Borrowed(text);
        }
        let ellipsis = if self.font().charmap().map('…').is_some() {
            "…"
        } else {
            "..."
        };
        let budget = max_width - self.text_width(ellipsis, size_px);
        if budget < 0.0 {
            return Cow::Borrowed("");
        }
        let mut width = 0.0;
        let mut end = 0;
        for (index, ch) in text.char_indices() {
            width += self.advance(ch, size_px);
            if width > budget {
                break;
            }
            end = index + ch.len_utf8();
        }
        Cow::Owned(format!("{}{ellipsis}", text[..end].trim_end()))
    }

    /// Ascent and descent (both positive, in pixels) at `size_px`.
    pub(crate) fn vertical_metrics(&self, size_px: f32) -> (f32, f32) {
        let metrics = self
            .font()
            .metrics(Size::new(size_px), LocationRef::default());
        if metrics.ascent - metrics.descent > 0.0 {
            (metrics.ascent, -metrics.descent)
        } else {
            (size_px * 0.8, size_px * 0.2)
        }
    }

    /// The whole-pixel advance of `ch` at `size_px`.
    fn advance(&self, ch: char, size_px: f32) -> f32 {
        self.glyph(ch, size_px, Antialias::Grayscale).advance
    }

    /// The rasterized glyph for `ch` at `size_px`, from the cache if present.
    pub(crate) fn glyph(&self, ch: char, size_px: f32, antialias: Antialias) -> Rc<Glyph> {
        let key = (ch, size_px.to_bits(), antialias);
        if let Some(glyph) = self.glyphs.borrow().get(&key) {
            return Rc::clone(glyph);
        }
        let glyph = Rc::new(self.rasterize(ch, size_px, antialias));
        self.glyphs.borrow_mut().insert(key, Rc::clone(&glyph));
        glyph
    }

    fn rasterize(&self, ch: char, size_px: f32, antialias: Antialias) -> Glyph {
        let font = self.font();
        let size = Size::new(size_px);
        let id = font.charmap().map(ch).unwrap_or(GlyphId::NOTDEF);
        let linear_advance = font
            .glyph_metrics(size, LocationRef::default())
            .advance_width(id)
            .unwrap_or_default();
        let mut pen = Pen::default();
        let mut advance = linear_advance;
        let outlines = font.outline_glyphs();
        if let Some(outline) = outlines.get(id) {
            let mut hinters = self.hinters.borrow_mut();
            let hinter = hinters.entry(size_px.to_bits()).or_insert_with(|| {
                let options = HintingOptions {
                    engine: Engine::Auto(None),
                    target: Target::Smooth {
                        mode: SmoothMode::Normal,
                        symmetric_rendering: true,
                        preserve_linear_metrics: false,
                    },
                };
                HintingInstance::new(&outlines, size, LocationRef::default(), options).ok()
            });
            let settings = hinter.as_ref().map_or_else(
                || DrawSettings::unhinted(size, LocationRef::default()),
                |hinter| DrawSettings::hinted(hinter, false),
            );
            if let Ok(adjusted) = outline.draw(settings, &mut pen) {
                advance = adjusted.advance_width.unwrap_or(linear_advance);
            } else {
                pen.0.clear();
            }
        }
        let advance = advance.round();
        match antialias {
            Antialias::Grayscale => rasterize_grayscale(&pen.0, advance),
            Antialias::Rgb | Antialias::Bgr => {
                rasterize_subpixel(&pen.0, advance, antialias == Antialias::Bgr)
            }
        }
    }
}

fn rasterize_grayscale(path: &[Command], advance: f32) -> Glyph {
    if path.is_empty() {
        return empty_glyph(advance);
    }
    let (coverage, placement) = Mask::new(path).render();
    Glyph {
        left: placement.left,
        top: placement.top,
        width: placement.width as usize,
        height: placement.height as usize,
        advance,
        coverage,
    }
}

/// Renders at three times the horizontal resolution, one sample per
/// subpixel, and filters the samples with [`LCD_FILTER`].
fn rasterize_subpixel(path: &[Command], advance: f32, bgr: bool) -> Glyph {
    if path.is_empty() {
        return empty_glyph(advance);
    }
    let (samples, placement) = Mask::new(path)
        .transform(Some(Transform::scale(3.0, 1.0)))
        .render();
    let sample_width = placement.width as usize;
    let height = placement.height as usize;
    // The filter spreads each sample two subpixels either way; widen the
    // bitmap to whole pixels covering that.
    let first = (placement.left - 2).div_euclid(3);
    let last = (placement.left + to_i32(sample_width) + 2).div_euclid(3);
    let width = usize::try_from(last - first + 1).unwrap_or_default();
    let mut filtered = vec![0u32; width * 3 * height];
    // Subpixel index in the bitmap of sample 0.
    let shift = placement.left - first * 3;
    for row in 0..height {
        let samples = &samples[row * sample_width..(row + 1) * sample_width];
        let out = &mut filtered[row * width * 3..(row + 1) * width * 3];
        for (column, &sample) in samples.iter().enumerate() {
            for (tap, &weight) in LCD_FILTER.iter().enumerate() {
                let subpixel = shift + to_i32(column) + to_i32(tap) - 2;
                if let Some(slot) = usize::try_from(subpixel).ok().and_then(|i| out.get_mut(i)) {
                    *slot += u32::from(sample) * weight;
                }
            }
        }
    }
    let mut coverage: Vec<u8> = filtered
        .into_iter()
        .map(|value| u8::try_from(value.div_ceil(256)).unwrap_or(u8::MAX))
        .collect();
    if bgr {
        for pixel in coverage.as_chunks_mut::<3>().0 {
            pixel.swap(0, 2);
        }
    }
    Glyph {
        left: first,
        top: placement.top,
        width,
        height,
        advance,
        coverage,
    }
}

fn empty_glyph(advance: f32) -> Glyph {
    Glyph {
        left: 0,
        top: 0,
        width: 0,
        height: 0,
        advance,
        coverage: Vec::new(),
    }
}

fn to_i32(value: usize) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
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
    fn advances_are_whole_pixels() {
        let fonts = Fonts::embedded();
        for size in [12.0, 16.0, 21.0, 32.0] {
            let width = fonts.text_width("a", size);
            assert!(width > 0.0 && width.fract() == 0.0, "{size}: {width}");
        }
    }

    #[test]
    fn hinting_snaps_vertical_stems() {
        // The stem of `l` is a single fully covered column at 16 px.
        let fonts = Fonts::embedded();
        let glyph = fonts.glyph('l', 16.0, Antialias::Grayscale);
        let middle = glyph.height / 2;
        let row = &glyph.coverage[middle * glyph.width..(middle + 1) * glyph.width];
        assert!(row.contains(&255), "{row:?}");
    }

    #[test]
    fn subpixel_glyphs_have_three_channels_and_the_same_advance() {
        let fonts = Fonts::embedded();
        let gray = fonts.glyph('m', 16.0, Antialias::Grayscale);
        for antialias in [Antialias::Rgb, Antialias::Bgr] {
            let lcd = fonts.glyph('m', 16.0, antialias);
            assert_eq!(lcd.coverage.len(), lcd.width * lcd.height * 3);
            assert_eq!(lcd.advance, gray.advance);
            assert_eq!(lcd.top, gray.top);
        }
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
