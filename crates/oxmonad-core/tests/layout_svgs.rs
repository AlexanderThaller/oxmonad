//! Generates the layout pictures in `docs/layouts/` from the real layout
//! code, and checks they are up to date.
//!
//! Regenerate with `OXMONAD_BLESS=1 cargo test -p oxmonad-core --test
//! layout_svgs`.

use std::{
    env,
    fmt::Write as _,
    fs,
    path::PathBuf,
};

use oxmonad_core::{
    Rect,
    layout::{
        LayoutRing,
        PREVIEW_AREA as SCREEN,
    },
};

/// Gap drawn around each window so neighbours stay distinguishable.
const INSET: i32 = 2;

fn docs_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/layouts")
}

/// `Tall Right` → `tall-right`.
fn slug(name: &str) -> String {
    name.to_lowercase().replace(' ', "-")
}

fn window(svg: &mut String, rect: Rect, label: &str, master: bool) {
    let fill = if master { "#9ec5f0" } else { "#dddddd" };
    let (x, y) = (rect.x + INSET, rect.y + INSET);
    let (w, h) = (rect.w - 2 * INSET, rect.h - 2 * INSET);
    let _ = writeln!(
        svg,
        r##"  <rect x="{x}" y="{y}" width="{w}" height="{h}" rx="3" fill="{fill}" stroke="#444444" stroke-width="1.5"/>"##
    );
    let (cx, cy) = (rect.x + rect.w / 2, rect.y + rect.h / 2);
    let _ = writeln!(
        svg,
        r##"  <text x="{cx}" y="{cy}" font-family="sans-serif" font-size="16" fill="#222222" text-anchor="middle" dominant-baseline="central">{label}</text>"##
    );
}

/// One layout's preview: windows `1`–`4` (window 1 is the master and
/// focused), exactly as the layout picker draws it.
fn render(ring: &LayoutRing) -> String {
    let output = ring.current().preview();
    let mut svg = String::new();
    let _ = writeln!(
        svg,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}" role="img" aria-label="{name} layout with four windows">"#,
        w = SCREEN.w,
        h = SCREEN.h,
        name = ring.name(),
    );
    let _ = writeln!(
        svg,
        r##"  <rect width="{}" height="{}" fill="#f4f4f4" stroke="#888888"/>"##,
        SCREEN.w, SCREEN.h
    );
    if let Some(tabs) = &output.tabs {
        for (index, tab) in tabs.tabs.iter().enumerate() {
            let fill = if index == 0 { "#9ec5f0" } else { "#dddddd" };
            let _ = writeln!(
                svg,
                r##"  <rect x="{}" y="{}" width="{}" height="{}" fill="{fill}" stroke="#444444"/>"##,
                tab.x, tab.y, tab.w, tab.h
            );
            let _ = writeln!(
                svg,
                r##"  <text x="{}" y="{}" font-family="sans-serif" font-size="12" fill="#222222" text-anchor="middle" dominant-baseline="central">{}</text>"##,
                tab.x + tab.w / 2,
                tab.y + tab.h / 2,
                index + 1
            );
        }
    }
    for &(index, rect) in &output.rects {
        window(&mut svg, rect, &(index + 1).to_string(), index == 0);
    }
    svg.push_str("</svg>\n");
    svg
}

#[test]
fn layout_pictures_are_up_to_date() {
    let dir = docs_dir();
    let bless = env::var_os("OXMONAD_BLESS").is_some();
    let mut ring = LayoutRing::standard(20);
    let mut stale = Vec::new();
    for _ in 0..ring.len() {
        let path = dir.join(format!("{}.svg", slug(&ring.name())));
        let svg = render(&ring);
        if bless {
            fs::create_dir_all(&dir).expect("docs/layouts");
            fs::write(&path, &svg).expect("write the picture");
        } else if fs::read_to_string(&path).ok().as_deref() != Some(svg.as_str()) {
            stale.push(path.display().to_string());
        }
        ring.next();
    }
    assert!(
        stale.is_empty(),
        "layout pictures are stale, regenerate with OXMONAD_BLESS=1: {stale:?}"
    );
}
