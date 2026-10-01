//! Icon decoding: `IconPixmap` data, PNG and SVG files, all converted to
//! premultiplied BGRA ([`TrayIcon`]).

use std::{
    collections::HashMap,
    fs,
    io::Cursor,
    path::{
        Path,
        PathBuf,
    },
};

use resvg::{
    tiny_skia::{
        FilterQuality,
        IntSize,
        Pixmap,
        PixmapPaint,
        Transform,
    },
    usvg,
};

use super::{
    TrayIcon,
    theme::IconThemes,
};

/// One `IconPixmap` entry: width, height and ARGB32 bytes in network
/// byte order (A, R, G, B), straight alpha.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RawPixmap {
    pub width: i32,
    pub height: i32,
    pub argb: Vec<u8>,
}

impl RawPixmap {
    /// The pixel dimensions, if positive and backed by enough data.
    fn size(&self) -> Option<(u32, u32)> {
        let width = u32::try_from(self.width).ok().filter(|w| *w > 0)?;
        let height = u32::try_from(self.height).ok().filter(|h| *h > 0)?;
        let needed = usize::try_from(u64::from(width) * u64::from(height) * 4).ok()?;
        (self.argb.len() >= needed).then_some((width, height))
    }
}

/// The pixmap to use for `target` pixels: the smallest one at least
/// that large, else the largest one. Malformed entries are ignored.
#[must_use]
pub fn choose_pixmap(pixmaps: &[RawPixmap], target: u32) -> Option<&RawPixmap> {
    let valid = pixmaps
        .iter()
        .filter_map(|pixmap| pixmap.size().map(|(w, h)| (w.max(h), pixmap)));
    let (big_enough, too_small): (Vec<_>, Vec<_>) = valid.partition(|(size, _)| *size >= target);
    big_enough
        .into_iter()
        .min_by_key(|(size, _)| *size)
        .or_else(|| too_small.into_iter().max_by_key(|(size, _)| *size))
        .map(|(_, pixmap)| pixmap)
}

/// `c * alpha / 255`, rounded.
#[expect(
    clippy::cast_possible_truncation,
    reason = "the product divided by 255 fits into a byte"
)]
fn premultiply(c: u8, alpha: u8) -> u8 {
    ((u16::from(c) * u16::from(alpha) + 127) / 255) as u8
}

/// Converts straight-alpha A, R, G, B bytes to premultiplied B, G, R, A.
#[must_use]
pub fn argb_to_bgra(argb: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(argb.len());
    for &[a, r, g, b] in argb.as_chunks::<4>().0 {
        out.extend_from_slice(&[premultiply(b, a), premultiply(g, a), premultiply(r, a), a]);
    }
    out
}

/// The best `IconPixmap` entry for `target` pixels, converted.
#[must_use]
pub fn pixmap_icon(pixmaps: &[RawPixmap], target: u32) -> Option<TrayIcon> {
    let pixmap = choose_pixmap(pixmaps, target)?;
    let (width, height) = pixmap.size()?;
    let len = usize::try_from(width * height * 4).ok()?;
    Some(TrayIcon {
        width,
        height,
        bgra: argb_to_bgra(&pixmap.argb[..len]),
    })
}

/// Swaps a tiny-skia pixmap's premultiplied RGBA into BGRA.
fn pixmap_to_icon(pixmap: &Pixmap) -> TrayIcon {
    let mut bgra = pixmap.data().to_vec();
    for pixel in bgra.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
    TrayIcon {
        width: pixmap.width(),
        height: pixmap.height(),
        bgra,
    }
}

/// Scales `pixmap` down to fit `size`×`size` (never up).
#[expect(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "icon dimensions are small"
)]
fn fit(pixmap: Pixmap, size: u32) -> Pixmap {
    let largest = pixmap.width().max(pixmap.height());
    if largest <= size || size == 0 {
        return pixmap;
    }
    let scale = size as f32 / largest as f32;
    let width = ((pixmap.width() as f32 * scale).round() as u32).max(1);
    let height = ((pixmap.height() as f32 * scale).round() as u32).max(1);
    let Some(mut scaled) = Pixmap::new(width, height) else {
        return pixmap;
    };
    let paint = PixmapPaint {
        quality: FilterQuality::Bicubic,
        ..PixmapPaint::default()
    };
    scaled.draw_pixmap(
        0,
        0,
        pixmap.as_ref(),
        &paint,
        Transform::from_scale(scale, scale),
        None,
    );
    scaled
}

/// Decodes a PNG into a premultiplied RGBA pixmap.
fn decode_png(data: &[u8]) -> Option<Pixmap> {
    let mut decoder = png::Decoder::new(Cursor::new(data));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().ok()?;
    let mut buffer = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buffer).ok()?;
    let pixels = usize::try_from(u64::from(info.width) * u64::from(info.height)).ok()?;
    let mut rgba = Vec::with_capacity(pixels * 4);
    let rows = buffer.chunks(info.line_size).take(info.height as usize);
    for row in rows {
        match info.color_type {
            png::ColorType::Grayscale => {
                for &v in row {
                    rgba.extend_from_slice(&[v, v, v, 255]);
                }
            }
            png::ColorType::GrayscaleAlpha => {
                for &[v, a] in row.as_chunks::<2>().0 {
                    let v = premultiply(v, a);
                    rgba.extend_from_slice(&[v, v, v, a]);
                }
            }
            png::ColorType::Rgb => {
                for &[r, g, b] in row.as_chunks::<3>().0 {
                    rgba.extend_from_slice(&[r, g, b, 255]);
                }
            }
            png::ColorType::Rgba => {
                for &[r, g, b, a] in row.as_chunks::<4>().0 {
                    rgba.extend_from_slice(&[
                        premultiply(r, a),
                        premultiply(g, a),
                        premultiply(b, a),
                        a,
                    ]);
                }
            }
            png::ColorType::Indexed => return None,
        }
    }
    Pixmap::from_vec(rgba, IntSize::from_wh(info.width, info.height)?)
}

/// Renders an SVG to fit `size`×`size`, centred.
#[expect(clippy::cast_precision_loss, reason = "icon dimensions are small")]
fn render_svg(data: &[u8], size: u32) -> Option<Pixmap> {
    let tree = usvg::Tree::from_data(data, &usvg::Options::default()).ok()?;
    let mut pixmap = Pixmap::new(size, size)?;
    let svg = tree.size();
    let scale = size as f32 / svg.width().max(svg.height());
    let dx = (size as f32 - svg.width() * scale) / 2.0;
    let dy = (size as f32 - svg.height() * scale) / 2.0;
    resvg::render(
        &tree,
        Transform::from_scale(scale, scale).post_translate(dx, dy),
        &mut pixmap.as_mut(),
    );
    Some(pixmap)
}

/// Loads a PNG or SVG file, scaled to fit `size`×`size` pixels (PNGs are
/// only scaled down).
#[must_use]
pub fn load_file(path: &Path, size: u32) -> Option<TrayIcon> {
    let data = fs::read(path)
        .inspect_err(|err| tracing::debug!(%err, path = %path.display(), "tray icon"))
        .ok()?;
    let pixmap = if data.starts_with(b"\x89PNG") {
        decode_png(&data).map(|pixmap| fit(pixmap, size))
    } else {
        render_svg(&data, size)
    };
    if pixmap.is_none() {
        tracing::debug!(path = %path.display(), "cannot decode tray icon");
    }
    pixmap.as_ref().map(pixmap_to_icon)
}

/// Resolves and decodes named icons, remembering the results (misses
/// too).
#[derive(Debug)]
pub struct IconCache {
    themes: IconThemes,
    loaded: HashMap<(String, Option<PathBuf>, u32), Option<TrayIcon>>,
}

/// Forget everything beyond this many cached icons.
const CACHE_LIMIT: usize = 256;

impl IconCache {
    pub fn new(themes: IconThemes) -> Self {
        Self {
            themes,
            loaded: HashMap::new(),
        }
    }

    /// The icon `name` (an absolute path or an icon theme name, looked up
    /// in `theme_path` first) at `size` pixels.
    pub fn named(&mut self, name: &str, theme_path: Option<&Path>, size: u32) -> Option<TrayIcon> {
        if name.is_empty() {
            return None;
        }
        let key = (name.to_owned(), theme_path.map(Path::to_path_buf), size);
        if let Some(icon) = self.loaded.get(&key) {
            return icon.clone();
        }
        let icon = self
            .themes
            .find(name, theme_path, size)
            .and_then(|path| load_file(&path, size));
        if icon.is_none() {
            tracing::debug!(name, "tray icon not found");
        }
        if self.loaded.len() >= CACHE_LIMIT {
            self.loaded.clear();
        }
        self.loaded.insert(key, icon.clone());
        icon
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixmap(size: i32) -> RawPixmap {
        RawPixmap {
            width: size,
            height: size,
            argb: vec![0xff; usize::try_from(size * size * 4).expect("size")],
        }
    }

    #[test]
    fn chooses_smallest_big_enough_else_largest() {
        let pixmaps = [pixmap(16), pixmap(64), pixmap(32), pixmap(48)];
        assert_eq!(choose_pixmap(&pixmaps, 32).map(|p| p.width), Some(32));
        assert_eq!(choose_pixmap(&pixmaps, 40).map(|p| p.width), Some(48));
        assert_eq!(choose_pixmap(&pixmaps, 100).map(|p| p.width), Some(64));
        assert_eq!(choose_pixmap(&[], 16), None);
    }

    #[test]
    fn ignores_malformed_pixmaps() {
        let short = RawPixmap {
            width: 128,
            height: 128,
            argb: vec![0; 16],
        };
        let negative = RawPixmap {
            width: -4,
            height: 4,
            argb: vec![0; 64],
        };
        let pixmaps = [short, negative, pixmap(8)];
        assert_eq!(choose_pixmap(&pixmaps, 64).map(|p| p.width), Some(8));
    }

    #[test]
    fn converts_argb_to_premultiplied_bgra() {
        // Opaque red, half-transparent white, transparent green.
        let argb = [255, 255, 0, 0, 128, 255, 255, 255, 0, 0, 255, 0];
        assert_eq!(
            argb_to_bgra(&argb),
            vec![0, 0, 255, 255, 128, 128, 128, 128, 0, 0, 0, 0]
        );
        let icon = pixmap_icon(
            &[RawPixmap {
                width: 1,
                height: 1,
                // Extra trailing data is cut off.
                argb: vec![255, 1, 2, 3, 9, 9],
            }],
            22,
        )
        .expect("icon");
        assert_eq!((icon.width, icon.height), (1, 1));
        assert_eq!(icon.bgra, vec![3, 2, 1, 255]);
    }

    #[test]
    fn decodes_png_and_svg() {
        let dir = std::env::temp_dir().join(format!("oxmonad-tray-icon-{}", std::process::id()));
        fs::create_dir_all(&dir).expect("mkdir");
        // 2x1 RGBA PNG: opaque blue, half-transparent red.
        let mut png_data = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut png_data, 2, 1);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().expect("header");
            writer
                .write_image_data(&[0, 0, 255, 255, 255, 0, 0, 128])
                .expect("data");
        }
        let png_path = dir.join("a.png");
        fs::write(&png_path, &png_data).expect("write");
        let icon = load_file(&png_path, 16).expect("png");
        assert_eq!((icon.width, icon.height), (2, 1));
        assert_eq!(icon.bgra, vec![255, 0, 0, 255, 0, 0, 128, 128]);

        let svg_path = dir.join("b.svg");
        fs::write(
            &svg_path,
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><rect width="10" height="10" fill="#00ff00"/></svg>"##,
        )
        .expect("write");
        let icon = load_file(&svg_path, 20).expect("svg");
        assert_eq!((icon.width, icon.height), (20, 20));
        assert_eq!(&icon.bgra[..4], &[0, 255, 0, 255]);
        fs::remove_dir_all(&dir).expect("cleanup");
    }
}
