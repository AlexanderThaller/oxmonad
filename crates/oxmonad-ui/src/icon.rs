//! Raster icons (e.g. system tray items) and their resampling.

use std::{
    fmt,
    sync::Arc,
};

use tiny_skia::{
    FilterQuality,
    Pixmap,
    PixmapPaint,
    Transform,
};

use crate::color::premultiply;

/// A device-pixel image: premultiplied ARGB8888 in little-endian byte order
/// (bytes `B, G, R, A` per pixel, rows tightly packed), the same layout as
/// [`Canvas`](crate::Canvas).
///
/// Cloning is cheap (the pixels are shared), so views holding icons can be
/// cloned and compared every frame.
#[derive(Clone)]
pub struct Icon {
    width: u32,
    height: u32,
    data: Arc<[u8]>,
}

impl fmt::Debug for Icon {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Icon")
            .field("width", &self.width)
            .field("height", &self.height)
            .finish_non_exhaustive()
    }
}

impl PartialEq for Icon {
    fn eq(&self, other: &Self) -> bool {
        self.width == other.width
            && self.height == other.height
            && (Arc::ptr_eq(&self.data, &other.data) || self.data == other.data)
    }
}

impl Eq for Icon {}

/// The number of bytes of a `width` × `height` image with 4 bytes per
/// pixel, if it is non-empty and fits in memory.
fn byte_len(width: u32, height: u32) -> Option<usize> {
    if width == 0 || height == 0 {
        return None;
    }
    usize::try_from(width)
        .ok()?
        .checked_mul(usize::try_from(height).ok()?)?
        .checked_mul(4)
}

impl Icon {
    /// An icon from premultiplied `B, G, R, A` bytes; `None` when the size is
    /// zero or `data` is not `width * height * 4` bytes long.
    #[must_use]
    pub fn from_premultiplied_bgra(
        width: u32,
        height: u32,
        data: impl Into<Arc<[u8]>>,
    ) -> Option<Self> {
        let data = data.into();
        (byte_len(width, height)? == data.len()).then_some(Self {
            width,
            height,
            data,
        })
    }

    /// An icon from straight (non-premultiplied) `R, G, B, A` bytes; `None`
    /// when the size is zero or `rgba` is not `width * height * 4` bytes long.
    #[must_use]
    pub fn from_straight_rgba(width: u32, height: u32, rgba: &[u8]) -> Option<Self> {
        Self::convert(width, height, rgba, |[r, g, b, a]| [r, g, b, a])
    }

    /// An icon from straight `A, R, G, B` bytes (32-bit ARGB in network byte
    /// order, the `StatusNotifierItem` `IconPixmap` format); `None` when the
    /// size is zero or `argb` is not `width * height * 4` bytes long.
    #[must_use]
    pub fn from_straight_argb_be(width: u32, height: u32, argb: &[u8]) -> Option<Self> {
        Self::convert(width, height, argb, |[a, r, g, b]| [r, g, b, a])
    }

    /// Premultiplies `source` after reordering each pixel to `R, G, B, A`
    /// with `to_rgba`.
    fn convert(
        width: u32,
        height: u32,
        source: &[u8],
        to_rgba: impl Fn([u8; 4]) -> [u8; 4],
    ) -> Option<Self> {
        if byte_len(width, height)? != source.len() {
            return None;
        }
        let data: Vec<u8> = source
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|&pixel| {
                let [r, g, b, a] = to_rgba(pixel);
                [premultiply(b, a), premultiply(g, a), premultiply(r, a), a]
            })
            .collect();
        Self::from_premultiplied_bgra(width, height, data)
    }

    /// Width in pixels.
    #[must_use]
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Height in pixels.
    #[must_use]
    pub fn height(&self) -> u32 {
        self.height
    }

    /// The premultiplied `B, G, R, A` bytes, `width * height * 4` long.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.data
    }

    /// The size this icon is drawn at inside a `box_size` device pixel
    /// square: the longer side fills the box, the aspect ratio is kept and
    /// neither side is below one pixel.
    pub(crate) fn fitted_size(&self, box_size: u32) -> (u32, u32) {
        let box_size = box_size.max(1);
        let (w, h) = (u64::from(self.width), u64::from(self.height));
        let long = w.max(h);
        let fit = |side: u64| {
            u32::try_from((side * u64::from(box_size) + long / 2) / long)
                .unwrap_or(box_size)
                .clamp(1, box_size)
        };
        (fit(w), fit(h))
    }

    /// The icon resampled to `width` × `height` pixels as a premultiplied
    /// RGBA pixmap: area-averaged when shrinking (so detail is not aliased
    /// away), bicubic when enlarging.
    pub(crate) fn resampled(&self, width: u32, height: u32) -> Option<Pixmap> {
        let mut source = Pixmap::new(self.width, self.height)?;
        for (dst, src) in source
            .data_mut()
            .as_chunks_mut::<4>()
            .0
            .iter_mut()
            .zip(self.data.as_chunks::<4>().0)
        {
            *dst = [src[2], src[1], src[0], src[3]];
        }
        if (width, height) == (self.width, self.height) {
            return Some(source);
        }
        if width <= self.width && height <= self.height {
            return area_average(&source, width, height);
        }
        let mut target = Pixmap::new(width, height)?;
        let paint = PixmapPaint {
            quality: FilterQuality::Bicubic,
            ..PixmapPaint::default()
        };
        let transform = Transform::from_scale(ratio(width, self.width), ratio(height, self.height));
        target.draw_pixmap(0, 0, source.as_ref(), &paint, transform, None);
        Some(target)
    }
}

/// `numerator / denominator` as `f32`.
#[expect(
    clippy::cast_precision_loss,
    reason = "icon dimensions are far below f32's exact integer range"
)]
fn ratio(numerator: u32, denominator: u32) -> f32 {
    numerator as f32 / denominator as f32
}

/// For each of `target` output cells along an axis of `source` input pixels:
/// the input pixels it covers and how much of each (weights sum to one).
fn box_weights(source: u32, target: u32) -> Vec<Vec<(usize, f32)>> {
    let step = f64::from(source) / f64::from(target);
    (0..target)
        .map(|cell| {
            let (start, end) = (f64::from(cell) * step, f64::from(cell + 1) * step);
            let mut weights = Vec::new();
            let mut pixel = start.floor();
            while pixel < end {
                let covered = (pixel + 1.0).min(end) - pixel.max(start);
                if covered > 0.0 {
                    #[expect(
                        clippy::cast_possible_truncation,
                        clippy::cast_sign_loss,
                        reason = "pixel is a non-negative index below the icon size"
                    )]
                    weights.push((pixel as usize, (covered / step) as f32));
                }
                pixel += 1.0;
            }
            weights
        })
        .collect()
}

/// Shrinks a premultiplied pixmap by averaging the source area under each
/// target pixel.
fn area_average(source: &Pixmap, width: u32, height: u32) -> Option<Pixmap> {
    let mut target = Pixmap::new(width, height)?;
    let source_width = source.width() as usize;
    let columns = box_weights(source.width(), width);
    let rows = box_weights(source.height(), height);
    let pixels = source.data().as_chunks::<4>().0;
    // Horizontal pass: one row of `width` accumulated pixels per source row.
    let horizontal: Vec<[f32; 4]> = pixels
        .chunks_exact(source_width)
        .flat_map(|row| {
            columns.iter().map(move |weights| {
                let mut sum = [0.0_f32; 4];
                for &(x, weight) in weights {
                    for (acc, channel) in sum.iter_mut().zip(row[x]) {
                        *acc += f32::from(channel) * weight;
                    }
                }
                sum
            })
        })
        .collect();
    let target_width = width as usize;
    for (y, weights) in rows.iter().enumerate() {
        for x in 0..target_width {
            let mut sum = [0.0_f32; 4];
            for &(source_y, weight) in weights {
                for (acc, channel) in sum.iter_mut().zip(horizontal[source_y * target_width + x]) {
                    *acc += channel * weight;
                }
            }
            let offset = (y * target_width + x) * 4;
            let alpha = sum[3].round().clamp(0.0, 255.0);
            for (index, value) in sum.iter().enumerate() {
                // Premultiplied channels never exceed alpha.
                let limit = if index == 3 { 255.0 } else { alpha };
                #[expect(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    reason = "clamped to 0..=255"
                )]
                let byte = value.round().clamp(0.0, limit) as u8;
                target.data_mut()[offset + index] = byte;
            }
        }
    }
    Some(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_mismatched_sizes() {
        assert!(Icon::from_premultiplied_bgra(2, 2, vec![0; 15]).is_none());
        assert!(Icon::from_premultiplied_bgra(0, 2, Vec::new()).is_none());
        assert!(Icon::from_straight_rgba(1, 1, &[0; 3]).is_none());
        assert!(Icon::from_premultiplied_bgra(2, 2, vec![0; 16]).is_some());
    }

    #[test]
    fn straight_rgba_is_premultiplied_into_bgra() {
        let icon = Icon::from_straight_rgba(1, 1, &[255, 0, 100, 128]);
        assert_eq!(
            icon.map(|icon| icon.as_bytes().to_vec()),
            Some(vec![50, 0, 128, 128])
        );
        let icon = Icon::from_straight_argb_be(1, 1, &[128, 255, 0, 100]);
        assert_eq!(
            icon.map(|icon| icon.as_bytes().to_vec()),
            Some(vec![50, 0, 128, 128])
        );
    }

    #[test]
    fn equality_compares_contents() {
        let a = Icon::from_premultiplied_bgra(1, 1, vec![1, 2, 3, 4]);
        let b = Icon::from_premultiplied_bgra(1, 1, vec![1, 2, 3, 4]);
        let c = Icon::from_premultiplied_bgra(1, 1, vec![1, 2, 3, 5]);
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_eq!(a.clone(), a);
    }

    #[test]
    fn fitted_size_keeps_aspect_ratio() {
        let icon = |w, h| Icon::from_premultiplied_bgra(w, h, vec![0; (w * h * 4) as usize]);
        assert_eq!(icon(64, 64).map(|i| i.fitted_size(16)), Some((16, 16)));
        assert_eq!(icon(32, 16).map(|i| i.fitted_size(16)), Some((16, 8)));
        assert_eq!(icon(1, 100).map(|i| i.fitted_size(16)), Some((1, 16)));
    }

    #[test]
    fn shrinking_averages_the_area() {
        // A 2×2 checkerboard of opaque white and transparent shrinks to one
        // half-covered pixel.
        let white = [255, 255, 255, 255];
        let clear = [0, 0, 0, 0];
        let data = [white, clear, clear, white].concat();
        let icon = Icon::from_premultiplied_bgra(2, 2, data);
        let pixmap = icon.and_then(|icon| icon.resampled(1, 1));
        assert_eq!(
            pixmap.map(|p| p.data().to_vec()),
            Some(vec![128, 128, 128, 128])
        );
    }
}
