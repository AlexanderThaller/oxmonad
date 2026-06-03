//! Headless golden-image snapshot testing.
//!
//! Renders a set of solid-color render elements with Smithay's software
//! ([`PixmanRenderer`]) backend — no GPU or display required, so it is fully
//! deterministic and CI-friendly — reads the pixels back via [`ExportMem`], and
//! compares them against a committed PNG under `tests/snapshots/`.
//!
//! Run `UPDATE_SNAPSHOTS=1 cargo test` to (re)generate the golden images, then
//! review the resulting PNG diff in git like any other change.
//!
//! Only compositor-drawn output (borders, backgrounds, mock window rectangles)
//! is snapshotted here; real client content (fonts, etc.) is non-deterministic
//! and is intentionally not golden-tested.

use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            Bind,
            ExportMem,
            Offscreen,
            damage::OutputDamageTracker,
            element::solid::SolidColorRenderElement,
            pixman::PixmanRenderer,
        },
    },
    utils::{
        Buffer as BufferCoord,
        Physical,
        Rectangle,
        Size,
        Transform,
    },
};

/// Background color the renderer clears to (matches the compositor).
const CLEAR_COLOR: [f32; 4] = [0.1, 0.1, 0.1, 1.0];

/// Per-channel tolerance when comparing against the golden image. Solid-color
/// software rendering is deterministic, but this absorbs any off-by-one from
/// differing libpixman versions without masking real (large) regressions.
const TOLERANCE: u8 = 4;

/// Renders `elements` to a `width`x`height` buffer and returns RGBA8 pixels.
fn render_to_rgba(width: u32, height: u32, elements: &[SolidColorRenderElement]) -> Vec<u8> {
    let w = i32::try_from(width).unwrap();
    let h = i32::try_from(height).unwrap();

    let mut renderer = PixmanRenderer::new().expect("create pixman renderer");
    let size = Size::<i32, BufferCoord>::from((w, h));
    let mut buffer = renderer
        .create_buffer(Fourcc::Argb8888, size)
        .expect("create offscreen buffer");
    let mut framebuffer = renderer.bind(&mut buffer).expect("bind offscreen buffer");

    let mut damage =
        OutputDamageTracker::new(Size::<i32, Physical>::from((w, h)), 1.0, Transform::Normal);
    damage
        .render_output(&mut renderer, &mut framebuffer, 0, elements, CLEAR_COLOR)
        .expect("render output");

    let region = Rectangle::<i32, BufferCoord>::from_size(size);
    let mapping = renderer
        .copy_framebuffer(&framebuffer, region, Fourcc::Argb8888)
        .expect("copy framebuffer");
    let bytes = renderer.map_texture(&mapping).expect("map texture");

    // Argb8888 is a little-endian 32-bit value, so the bytes are B, G, R, A;
    // swap to the R, G, B, A order the `image` crate expects.
    let mut rgba = Vec::with_capacity(bytes.len());
    for pixel in bytes.chunks_exact(4) {
        rgba.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
    }
    rgba
}

/// Renders `elements` and asserts they match the golden image `name`.
///
/// With `UPDATE_SNAPSHOTS` set (or when the golden is missing) the golden is
/// written instead. On mismatch an `<name>.actual.png` is written next to the
/// golden for inspection.
pub fn assert_snapshot(name: &str, width: u32, height: u32, elements: &[SolidColorRenderElement]) {
    let rgba = render_to_rgba(width, height, elements);
    let actual = image::RgbaImage::from_raw(width, height, rgba).expect("build image from pixels");

    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots");
    std::fs::create_dir_all(&dir).expect("create snapshot directory");
    let golden = dir.join(format!("{name}.png"));

    let updating = std::env::var_os("UPDATE_SNAPSHOTS").is_some();
    if updating || !golden.exists() {
        actual.save(&golden).expect("write golden image");
        assert!(
            updating,
            "created new snapshot '{name}'; re-run the tests to verify it",
        );
        return;
    }

    let expected = image::open(&golden).expect("open golden image").to_rgba8();
    assert_eq!(
        (expected.width(), expected.height()),
        (actual.width(), actual.height()),
        "snapshot '{name}': dimensions differ from golden",
    );

    let differing = expected
        .pixels()
        .zip(actual.pixels())
        .filter(|(e, a)| {
            e.0.iter()
                .zip(a.0.iter())
                .any(|(x, y)| x.abs_diff(*y) > TOLERANCE)
        })
        .count();

    if differing > 0 {
        let actual_path = dir.join(format!("{name}.actual.png"));
        actual.save(&actual_path).expect("write actual image");
        panic!(
            "snapshot '{name}': {differing} pixel(s) exceed tolerance {TOLERANCE}; wrote {}",
            actual_path.display(),
        );
    }
}
