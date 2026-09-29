//! The output buffer and the painter that fills it.

use std::fmt;

use tiny_skia::{
    Paint,
    Pixmap,
    Rect,
    Transform,
};

use crate::{
    Color,
    Fonts,
    color::premultiply,
};

/// A rendered widget: premultiplied ARGB8888 pixels in little-endian byte
/// order (bytes `B, G, R, A` per pixel), i.e. exactly the `wl_shm` `argb8888`
/// format. Dimensions are device pixels (logical size × scale).
#[derive(Clone, PartialEq, Eq)]
pub struct Canvas {
    width: u32,
    height: u32,
    scale: u32,
    data: Vec<u8>,
}

impl fmt::Debug for Canvas {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Canvas")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("scale", &self.scale)
            .finish_non_exhaustive()
    }
}

impl Canvas {
    /// Width in device pixels.
    #[must_use]
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Height in device pixels.
    #[must_use]
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Bytes per row (`width * 4`; rows are tightly packed).
    #[must_use]
    pub fn stride(&self) -> u32 {
        self.width * 4
    }

    /// The integer scale this was rendered at; set it as the surface's buffer
    /// scale.
    #[must_use]
    pub fn scale(&self) -> u32 {
        self.scale
    }

    /// The pixel bytes, `stride() * height()` long.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.data
    }

    /// Consumes the canvas, returning its pixel bytes.
    #[must_use]
    pub fn into_bytes(self) -> Vec<u8> {
        self.data
    }

    /// The premultiplied `[b, g, r, a]` bytes of the pixel at (`x`, `y`).
    #[must_use]
    pub fn pixel(&self, x: u32, y: u32) -> Option<[u8; 4]> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let offset = (y * self.stride() + x * 4) as usize;
        self.data[offset..offset + 4].try_into().ok()
    }
}

/// Draws rectangles and text into a pixmap in device pixels.
pub(crate) struct Painter {
    pixmap: Pixmap,
    scale: u32,
}

impl Painter {
    /// A transparent painter of `width` × `height` logical pixels (at least
    /// 1 × 1) rendered at `scale` (at least 1).
    pub fn new(width: u32, height: u32, scale: u32) -> Self {
        let scale = scale.max(1);
        let pixmap = Pixmap::new(width.max(1) * scale, height.max(1) * scale)
            .expect("the canvas size is non-zero and fits in memory");
        Self { pixmap, scale }
    }

    pub fn scale(&self) -> u32 {
        self.scale
    }

    pub fn width(&self) -> i32 {
        to_i32(self.pixmap.width())
    }

    pub fn height(&self) -> i32 {
        to_i32(self.pixmap.height())
    }

    /// Fills a rectangle given in device pixels (clipped to the canvas).
    pub fn fill_rect(&mut self, x: i32, y: i32, width: i32, height: i32, color: Color) {
        if width <= 0 || height <= 0 {
            return;
        }
        let Some(rect) = Rect::from_xywh(to_f32(x), to_f32(y), to_f32(width), to_f32(height))
        else {
            return;
        };
        let mut paint = Paint::default();
        paint.set_color_rgba8(color.r, color.g, color.b, color.a);
        paint.anti_alias = false;
        self.pixmap
            .fill_rect(rect, &paint, Transform::identity(), None);
    }

    /// Draws a `thickness`-pixel border along the inside of a rectangle.
    pub fn stroke_rect(
        &mut self,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        thickness: i32,
        color: Color,
    ) {
        let t = thickness.min(width / 2).min(height / 2).max(0);
        if t <= 0 {
            return;
        }
        self.fill_rect(x, y, width, t, color);
        self.fill_rect(x, y + height - t, width, t, color);
        self.fill_rect(x, y + t, t, height - 2 * t, color);
        self.fill_rect(x + width - t, y + t, t, height - 2 * t, color);
    }

    /// Draws `text` starting at pen position `x`, vertically centered in the
    /// band `top..top + band`, clipped to `clip_x..clip_right`. All values
    /// are device pixels; `size_px` is the device pixel font size.
    #[expect(
        clippy::too_many_arguments,
        reason = "a flat text-drawing primitive; bundling these adds noise"
    )]
    pub fn draw_text(
        &mut self,
        fonts: &Fonts,
        text: &str,
        x: f32,
        top: i32,
        band: i32,
        size_px: f32,
        color: Color,
        clip: (i32, i32),
    ) {
        let baseline = baseline(fonts, top, band, size_px);
        let mut pen = x;
        for ch in text.chars() {
            let glyph = fonts.glyph(ch, size_px);
            let metrics = glyph.metrics;
            let origin_x = round_i32(pen) + metrics.xmin;
            let origin_y = baseline - to_i32(metrics.height) - metrics.ymin;
            for row in 0..metrics.height {
                let coverage_row = &glyph.coverage[row * metrics.width..(row + 1) * metrics.width];
                for (column, &coverage) in coverage_row.iter().enumerate() {
                    let px = origin_x + to_i32(column);
                    if px >= clip.0 && px < clip.1 {
                        self.blend(px, origin_y + to_i32(row), color, coverage);
                    }
                }
            }
            pen += metrics.advance_width;
        }
    }

    /// Composites `color` at `coverage` over one pixel (source-over).
    fn blend(&mut self, x: i32, y: i32, color: Color, coverage: u8) {
        let (Ok(x), Ok(y)) = (u32::try_from(x), u32::try_from(y)) else {
            return;
        };
        if coverage == 0 || x >= self.pixmap.width() || y >= self.pixmap.height() {
            return;
        }
        let alpha = premultiply(color.a, coverage);
        let source = [
            premultiply(color.r, alpha),
            premultiply(color.g, alpha),
            premultiply(color.b, alpha),
            alpha,
        ];
        let offset = ((y * self.pixmap.width() + x) * 4) as usize;
        let pixel = &mut self.pixmap.data_mut()[offset..offset + 4];
        for (dst, src) in pixel.iter_mut().zip(source) {
            *dst = src + premultiply(*dst, 255 - alpha);
        }
    }

    /// Converts the RGBA pixmap into a BGRA [`Canvas`].
    pub fn finish(self) -> Canvas {
        let (width, height) = (self.pixmap.width(), self.pixmap.height());
        let mut data = self.pixmap.take();
        for pixel in data.as_chunks_mut::<4>().0 {
            pixel.swap(0, 2);
        }
        Canvas {
            width,
            height,
            scale: self.scale,
            data,
        }
    }
}

/// The baseline y that vertically centers a line of text in `top..top+band`.
pub(crate) fn baseline(fonts: &Fonts, top: i32, band: i32, size_px: f32) -> i32 {
    let (ascent, descent) = fonts.vertical_metrics(size_px);
    top + round_i32((to_f32(band) - (ascent + descent)) / 2.0 + ascent)
}

/// Converts a device pixel coordinate to `f32`.
#[expect(
    clippy::cast_precision_loss,
    reason = "device pixel coordinates are far below f32's exact integer range"
)]
pub(crate) fn to_f32(value: i32) -> f32 {
    value as f32
}

/// Rounds a device pixel coordinate to the nearest integer.
#[expect(
    clippy::cast_possible_truncation,
    reason = "device pixel coordinates fit in i32"
)]
pub(crate) fn round_i32(value: f32) -> i32 {
    value.round() as i32
}

/// Converts a pixel count to `i32`, saturating.
pub(crate) fn to_i32<T: TryInto<i32>>(value: T) -> i32 {
    value.try_into().unwrap_or(i32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_are_premultiplied_bgra() {
        let mut painter = Painter::new(1, 1, 1);
        painter.fill_rect(0, 0, 1, 1, Color::rgba(255, 0, 0, 128));
        let canvas = painter.finish();
        assert_eq!(canvas.as_bytes(), &[0, 0, 128, 128]);
        assert_eq!(canvas.stride(), 4);
    }

    #[test]
    fn glyph_blending_is_premultiplied_bgra() {
        let mut painter = Painter::new(1, 1, 1);
        painter.fill_rect(0, 0, 1, 1, Color::rgb(0, 0, 255));
        painter.blend(0, 0, Color::rgb(255, 0, 0), 128);
        let canvas = painter.finish();
        assert_eq!(canvas.pixel(0, 0), Some([127, 0, 128, 255]));
    }

    #[test]
    fn scale_multiplies_the_pixel_size() {
        let canvas = Painter::new(10, 3, 2).finish();
        assert_eq!(
            (canvas.width(), canvas.height(), canvas.scale()),
            (20, 6, 2)
        );
        assert_eq!(canvas.as_bytes().len(), 20 * 6 * 4);
    }
}
