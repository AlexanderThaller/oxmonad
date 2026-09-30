//! Layout geometry tests. The expected rectangles were computed with a
//! direct transcription of the Haskell formulas (`tile`, `splitVertically`,
//! `splitHorizontallyBy`, `tile3`, `split3HorizontallyBy`, Grid's `arrange`
//! and `chop`, `mirrorRect`, `reflectRect Horiz`) using exact rationals;
//! a few are derived by hand in comments.

use super::*;

mod ring_geometry;

pub(super) const FHD: Rect = Rect::new(0, 0, 1920, 1080);
pub(super) const OFFSET: Rect = Rect::new(1920, 24, 2560, 1416);

pub(super) const fn r(x: i32, y: i32, w: i32, h: i32) -> Rect {
    Rect::new(x, y, w, h)
}

fn ring_at(index: usize) -> LayoutRing {
    let mut ring = LayoutRing::standard(20);
    for _ in 0..index {
        ring.next();
    }
    ring
}

fn indexed(rects: &[Rect]) -> Vec<(usize, Rect)> {
    rects.iter().copied().enumerate().collect()
}

/// Checks every window count of a tiling ring entry, for each focus.
pub(super) fn assert_ring_entry(index: usize, area: Rect, cases: &[(usize, Vec<Rect>)]) {
    let ring = ring_at(index);
    for (n, expected) in cases {
        for focus in 0..*n {
            assert_eq!(
                ring.arrange(area, *n, focus),
                LayoutOutput {
                    rects: indexed(expected),
                    tabs: None,
                },
                "{} with {n} windows, focus {focus}",
                ring.name()
            );
        }
    }
}

fn tall() -> Tall {
    Tall::new(1, Ratio::new(3, 100), Ratio::new(3, 5))
}

fn ctx(focus: usize, count: usize) -> LayoutContext {
    LayoutContext {
        focus: Some(focus),
        count,
    }
}

#[test]
fn names_describe_where_the_master_goes() {
    let mut ring = LayoutRing::standard(20);
    let mut names = Vec::new();
    for _ in 0..ring.len() {
        names.push(ring.name());
        ring.next();
    }
    assert_eq!(
        names,
        [
            "Tall Right",
            "Tall",
            "Wide",
            "Full",
            "Tabs",
            "Grid Rows",
            "Grid",
            "Three Columns",
            "Three Columns Right",
            "Corner",
        ]
    );
}

#[test]
fn only_no_borders_full_drops_borders() {
    let mut ring = LayoutRing::standard(20);
    let mut borders = Vec::new();
    for _ in 0..ring.len() {
        borders.push(ring.has_borders());
        ring.next();
    }
    assert_eq!(
        borders,
        [true, true, true, false, true, true, true, true, true, true]
    );
    assert!(!Layout::Reflect(Box::new(Layout::NoBorders(Box::new(Layout::Grid)))).has_borders());
}

#[test]
fn empty_ring_falls_back_to_full() {
    let ring = LayoutRing::new(Vec::new());
    assert_eq!(ring.len(), 1);
    assert!(!ring.is_empty());
    assert_eq!(ring.current(), &Layout::Full);
}

#[test]
fn zero_windows_arrange_to_nothing() {
    let mut ring = LayoutRing::standard(20);
    for _ in 0..ring.len() {
        assert_eq!(ring.arrange(FHD, 0, 0), LayoutOutput::default());
        ring.next();
    }
}

#[test]
fn tall_by_hand() {
    // Master: floor (1920 * 3/5) = 1152. Stack of 2: 1080 `div` 2 = 540.
    assert_eq!(
        tall().arrange(FHD, 3),
        [
            r(0, 0, 1152, 1080),
            r(1152, 0, 768, 540),
            r(1152, 540, 768, 540)
        ]
    );
    // Uneven stack of 7: each row is the remaining height divided by the
    // remaining count: 1080/7 = 154 (rest 926), 926/6 = 154, 772/5 = 154,
    // 618/4 = 154, 464/3 = 154, 310/2 = 155, and the last row gets 155.
    let rects = tall().arrange(FHD, 8);
    assert_eq!(rects[1], r(1152, 0, 768, 154));
    assert_eq!(rects[5], r(1152, 616, 768, 154));
    assert_eq!(rects[6], r(1152, 770, 768, 155));
    assert_eq!(rects[7], r(1152, 925, 768, 155));
}

#[test]
fn full_shows_only_the_focused_window() {
    let ring = ring_at(3);
    assert_eq!(
        ring.arrange(FHD, 3, 1),
        LayoutOutput {
            rects: vec![(1, FHD)],
            tabs: None,
        }
    );
    assert_eq!(ring.arrange(OFFSET, 1, 0).rects, vec![(0, OFFSET)]);
}

#[test]
fn tabbed_hides_strip_for_a_single_window() {
    let ring = ring_at(4);
    assert_eq!(
        ring.arrange(FHD, 1, 0),
        LayoutOutput {
            rects: vec![(0, FHD)],
            tabs: None,
        }
    );
}

#[test]
fn tabbed_geometry() {
    let ring = ring_at(4);
    // Tab i starts at x + (w * i) `div` n: 1920/3 = 640 exactly.
    assert_eq!(
        ring.arrange(FHD, 3, 2),
        LayoutOutput {
            rects: vec![(2, r(0, 20, 1920, 1060))],
            tabs: Some(TabGeometry {
                strip: r(0, 0, 1920, 20),
                tabs: vec![r(0, 0, 640, 20), r(640, 0, 640, 20), r(1280, 0, 640, 20)],
            }),
        }
    );
    // 2560 * i / 3 = 0, 853, 1706, 2560: widths 853, 853, 854.
    assert_eq!(
        ring.arrange(OFFSET, 3, 0),
        LayoutOutput {
            rects: vec![(0, r(1920, 44, 2560, 1396))],
            tabs: Some(TabGeometry {
                strip: r(1920, 24, 2560, 20),
                tabs: vec![
                    r(1920, 24, 853, 20),
                    r(2773, 24, 853, 20),
                    r(3626, 24, 854, 20),
                ],
            }),
        }
    );
    // Five tabs, 1920/5 = 384.
    let five = ring.arrange(FHD, 5, 4).tabs.expect("tabs shown");
    assert_eq!(five.tabs.len(), 5);
    assert_eq!(five.tabs[4], r(1536, 0, 384, 20));
}

#[test]
fn reflected_tabbed_mirrors_the_tabs() {
    let layout = Layout::Reflect(Box::new(Layout::Tabbed { tab_height: 20 }));
    let tabs = layout.arrange(FHD, 2, 0).tabs.expect("tabs shown");
    assert_eq!(tabs.tabs, vec![r(960, 0, 960, 20), r(0, 0, 960, 20)]);
    assert_eq!(tabs.strip, r(0, 0, 1920, 20));
}

#[test]
fn tall_shrink_and_expand_clamp() {
    let mut t = tall();
    for _ in 0..3 {
        assert!(t.handle(LayoutMessage::Shrink, ctx(0, 3)));
    }
    assert_eq!(t.frac, Ratio::new(51, 100));
    // floor (1920 * 51/100) = 979.
    assert_eq!(
        t.arrange(FHD, 3),
        [
            r(0, 0, 979, 1080),
            r(979, 0, 941, 540),
            r(979, 540, 941, 540)
        ]
    );
    for _ in 0..40 {
        t.handle(LayoutMessage::Shrink, ctx(0, 3));
    }
    assert_eq!(t.frac, Ratio::ZERO);
    assert_eq!(t.arrange(FHD, 2), [r(0, 0, 0, 1080), r(0, 0, 1920, 1080)]);
    for _ in 0..40 {
        t.handle(LayoutMessage::Expand, ctx(0, 3));
    }
    assert_eq!(t.frac, Ratio::ONE);
    assert_eq!(
        t.arrange(FHD, 2),
        [r(0, 0, 1920, 1080), r(1920, 0, 0, 1080)]
    );
}

#[test]
fn tall_ignores_messages_without_tiled_focus() {
    let mut t = tall();
    let none = LayoutContext {
        focus: None,
        count: 3,
    };
    for msg in [
        LayoutMessage::Shrink,
        LayoutMessage::Expand,
        LayoutMessage::MirrorShrink,
        LayoutMessage::MirrorExpand,
        LayoutMessage::IncMasterN(1),
    ] {
        assert!(!t.handle(msg, none));
    }
    assert_eq!(t, tall());
    let mut ring = LayoutRing::standard(20);
    assert!(!ring.handle(LayoutMessage::Expand, none));
    assert_eq!(ring, LayoutRing::standard(20));
}

#[test]
fn tall_mirror_shrink_on_a_middle_stack_window() {
    // Focus index 1 of 3 (first stack window): not nmaster-1 (0) nor
    // total-1 (2), so pos = 1 and its factor grows to 103/100.
    let mut t = tall();
    assert!(t.handle(LayoutMessage::MirrorShrink, ctx(1, 3)));
    assert_eq!(t.slaves, vec![Ratio::ONE, Ratio::new(103, 100), Ratio::ONE]);
    // Stack of 2: floor (540 * 103/100) = 556, the last row gets 524.
    assert_eq!(
        t.arrange(FHD, 3),
        [
            r(0, 0, 1152, 1080),
            r(1152, 0, 768, 556),
            r(1152, 556, 768, 524)
        ]
    );
    assert!(t.handle(LayoutMessage::MirrorExpand, ctx(1, 3)));
    assert_eq!(t.slaves, vec![Ratio::ONE; 3]);
}

#[test]
fn tall_mirror_expand_on_the_last_window_resizes_the_one_above() {
    // Focus 3 of 4 is total-1, so pos = 2.
    let mut t = tall();
    assert!(t.handle(LayoutMessage::MirrorExpand, ctx(3, 4)));
    assert_eq!(
        t.slaves,
        vec![Ratio::ONE, Ratio::ONE, Ratio::new(97, 100), Ratio::ONE]
    );
    // Stack of 3 in 1080: 360, then floor (360 * 97/100) = 349, then 371.
    assert_eq!(
        t.arrange(FHD, 4),
        [
            r(0, 0, 1152, 1080),
            r(1152, 0, 768, 360),
            r(1152, 360, 768, 349),
            r(1152, 709, 768, 371),
        ]
    );
}

#[test]
fn tall_mirror_resize_on_the_master_is_a_no_op() {
    // Focus 0 with nmaster 1: pos = -1, which never matches, so the list is
    // only padded to `total`.
    let mut t = tall();
    assert!(t.handle(LayoutMessage::MirrorShrink, ctx(0, 3)));
    assert_eq!(t.slaves, vec![Ratio::ONE; 3]);
    assert_eq!(t.arrange(FHD, 3), tall().arrange(FHD, 3));
}

#[test]
fn tall_mirror_resize_truncates_to_the_window_count() {
    let mut t = tall();
    t.handle(LayoutMessage::MirrorShrink, ctx(1, 5));
    assert_eq!(t.slaves.len(), 5);
    t.handle(LayoutMessage::MirrorShrink, ctx(1, 2));
    // Focus 1 of 2 is the last window: pos 0, the master's factor.
    assert_eq!(t.slaves, vec![Ratio::new(103, 100), Ratio::new(103, 100)]);
}

#[test]
fn tall_negative_factor_takes_the_whole_column() {
    // A negative floor wraps in Haskell's Word32, so `min sh` picks sh.
    let mut t = tall();
    t.slaves = vec![Ratio::ONE, Ratio::new(-1, 10)];
    assert_eq!(
        t.arrange(FHD, 3),
        [
            r(0, 0, 1152, 1080),
            r(1152, 0, 768, 1080),
            r(1152, 1080, 768, 0)
        ]
    );
}

#[test]
fn tall_repeated_mirror_shrink_is_capped_by_the_column() {
    let mut t = tall();
    for _ in 0..100 {
        t.handle(LayoutMessage::MirrorShrink, ctx(1, 3));
    }
    // Factor 4: floor (540 * 4) = 2160, capped at the column height.
    assert_eq!(
        t.arrange(FHD, 3),
        [
            r(0, 0, 1152, 1080),
            r(1152, 0, 768, 1080),
            r(1152, 1080, 768, 0)
        ]
    );
}

#[test]
fn tall_inc_master() {
    let mut t = tall();
    assert!(t.handle(LayoutMessage::IncMasterN(1), ctx(0, 3)));
    assert_eq!(
        t.arrange(FHD, 3),
        [
            r(0, 0, 1152, 540),
            r(0, 540, 1152, 540),
            r(1152, 0, 768, 1080)
        ]
    );
    t.handle(LayoutMessage::IncMasterN(-5), ctx(0, 3));
    assert_eq!(t.nmaster, 0);
    // nmaster 0: one column for everything.
    assert_eq!(
        t.arrange(FHD, 2),
        [r(0, 0, 1920, 540), r(0, 540, 1920, 540)]
    );
    // Two masters, one window: the master column takes the whole area.
    t.handle(LayoutMessage::IncMasterN(2), ctx(0, 1));
    assert_eq!(t.arrange(FHD, 1), [FHD]);
}

#[test]
fn three_col_messages() {
    let mut cols = ThreeColumns::new(1, Ratio::new(3, 100), Ratio::new(1, 2));
    assert!(cols.handle(LayoutMessage::Expand));
    assert_eq!(cols.frac, Ratio::new(53, 100));
    assert!(!cols.handle(LayoutMessage::MirrorExpand));
    assert!(!cols.handle(LayoutMessage::MirrorShrink));
    for _ in 0..100 {
        cols.handle(LayoutMessage::Shrink);
    }
    assert_eq!(cols.frac, Ratio::new(-1, 2));
    // Three columns: f = 1 + 2 * (-1/2) = 0, so the master is 0 wide and
    // the stack splits the full width: ceiling (1920 / 2) = 960.
    assert_eq!(
        cols.arrange(FHD, 3),
        [r(0, 0, 0, 1080), r(0, 0, 960, 1080), r(960, 0, 960, 1080)]
    );
    // Two columns: f = 1 + (-1/2) = 1/2.
    assert_eq!(
        cols.arrange(FHD, 2),
        [r(0, 0, 960, 1080), r(960, 0, 960, 1080)]
    );
    for _ in 0..100 {
        cols.handle(LayoutMessage::Expand);
    }
    assert_eq!(cols.frac, Ratio::ONE);
}

#[test]
fn three_col_handles_messages_without_tiled_focus() {
    let mut ring = ring_at(7);
    let none = LayoutContext::default();
    assert!(ring.handle(LayoutMessage::IncMasterN(1), none));
    assert_eq!(
        ring.arrange(FHD, 5, 0).rects,
        indexed(&[
            r(0, 0, 960, 540),
            r(0, 540, 960, 540),
            r(960, 0, 480, 540),
            r(960, 540, 480, 540),
            r(1440, 0, 480, 1080),
        ])
    );
}

#[test]
fn three_col_odd_width_uses_ceiling() {
    // r1w = ceiling (1001 / 2) = 501, r2w = ceiling (500 / 2) = 250.
    let cols = ThreeColumns::new(1, Ratio::new(3, 100), Ratio::new(1, 2));
    assert_eq!(
        cols.arrange(r(0, 0, 1001, 100), 3),
        [r(0, 0, 501, 100), r(501, 0, 250, 100), r(751, 0, 250, 100)]
    );
}

#[test]
fn grid_more_counts() {
    let ring = ring_at(6);
    assert_eq!(
        ring.arrange(FHD, 4, 0).rects,
        indexed(&[
            r(0, 0, 960, 540),
            r(0, 540, 960, 540),
            r(960, 0, 960, 540),
            r(960, 540, 960, 540),
        ])
    );
    // 7 windows: ncols = round (sqrt 7) = 3, 2 per column, the last column
    // gets the extra window.
    assert_eq!(
        ring.arrange(FHD, 7, 0).rects,
        indexed(&[
            r(0, 0, 640, 540),
            r(0, 540, 640, 540),
            r(640, 0, 640, 540),
            r(640, 540, 640, 540),
            r(1280, 0, 640, 360),
            r(1280, 360, 640, 360),
            r(1280, 720, 640, 360),
        ])
    );
}

#[test]
fn grid_ignores_messages() {
    let mut ring = ring_at(6);
    let before = ring.clone();
    for msg in [
        LayoutMessage::Shrink,
        LayoutMessage::Expand,
        LayoutMessage::MirrorShrink,
        LayoutMessage::MirrorExpand,
        LayoutMessage::IncMasterN(1),
    ] {
        assert!(!ring.handle(msg, ctx(0, 3)));
    }
    assert_eq!(ring, before);
}

#[test]
fn ring_keeps_per_entry_state() {
    let mut ring = LayoutRing::standard(20);
    // Shrink the reflected tall layout, then cycle all the way round.
    assert!(ring.handle(LayoutMessage::Shrink, ctx(0, 2)));
    ring.next();
    // The plain tall entry is untouched.
    assert_eq!(
        ring.arrange(FHD, 2, 0).rects,
        indexed(&[r(0, 0, 1152, 1080), r(1152, 0, 768, 1080)])
    );
    for _ in 1..ring.len() {
        ring.next();
    }
    assert_eq!(ring.index(), 0);
    // floor (1920 * 57/100) = 1094; reflected: x = 1920 - 1094 = 826.
    assert_eq!(
        ring.arrange(FHD, 2, 0).rects,
        indexed(&[r(826, 0, 1094, 1080), r(0, 0, 826, 1080)])
    );
}

#[test]
fn mirror_tall_messages_pass_through() {
    let mut ring = ring_at(2);
    assert!(ring.handle(LayoutMessage::Expand, ctx(0, 2)));
    // floor (1080 * 63/100) = 680.
    assert_eq!(
        ring.arrange(FHD, 2, 0).rects,
        indexed(&[r(0, 0, 1920, 680), r(0, 680, 1920, 400)])
    );
}

fn tall_of(ring: &LayoutRing, index: usize) -> Tall {
    let mut ring = ring.clone();
    while ring.index() != index {
        ring.next();
    }
    let mut layout = ring.current();
    if let Layout::Reflect(inner) | Layout::Mirror(inner) = layout {
        layout = inner.as_ref();
    }
    let Layout::Tall(tall) = layout else {
        panic!("not tall: {layout:?}");
    };
    tall.clone()
}

#[test]
fn restore_keeps_a_compatible_ring() {
    let template = LayoutRing::standard(20);
    let mut saved = LayoutRing::standard(30);
    let ctx = LayoutContext {
        focus: Some(1),
        count: 3,
    };
    saved.handle(LayoutMessage::Shrink, ctx);
    saved.handle(LayoutMessage::IncMasterN(1), ctx);
    saved.handle(LayoutMessage::MirrorExpand, ctx);
    saved.next();
    saved.next();
    saved.handle(LayoutMessage::Expand, ctx);
    let restored = template.restore(&saved);
    assert_eq!(restored.index(), 2);
    assert_eq!(tall_of(&restored, 0), tall_of(&saved, 0));
    assert_eq!(tall_of(&restored, 2), tall_of(&saved, 2));
    assert_ne!(tall_of(&restored, 0), tall_of(&template, 0));
    // Tabbed takes the configured tab height.
    let mut tabbed = restored.clone();
    while tabbed.name() != "Tabs" {
        tabbed.next();
    }
    assert_eq!(tabbed.current(), &Layout::Tabbed { tab_height: 20 });
}

#[test]
fn restore_falls_back_to_the_template() {
    let template = LayoutRing::standard(20);
    // A different ring: the template, on the layout with the same name.
    let mut saved = LayoutRing::new(vec![Layout::Full, Layout::Grid]);
    saved.next();
    let restored = template.restore(&saved);
    assert_eq!(restored.name(), "Grid");
    assert_eq!(restored.index(), 6);
    assert_eq!(restored.len(), template.len());
    // `Full` matches `noBorders Full` (same name).
    let saved = LayoutRing::new(vec![Layout::Full]);
    assert_eq!(template.restore(&saved).index(), 3);
    // No layout with that name: the first one.
    let saved = LayoutRing::new(vec![Layout::Mirror(Box::new(Layout::Full))]);
    assert_eq!(template.restore(&saved), template);
    // Same length, different layouts.
    let saved = LayoutRing::new(vec![Layout::Grid; template.len()]);
    let restored = template.restore(&saved);
    assert_eq!(restored.index(), 6);
    assert_eq!(tall_of(&restored, 0), tall_of(&template, 0));
}

#[test]
fn restore_rejects_an_out_of_range_index() {
    let template = LayoutRing::standard(20);
    let mut json = serde_json::to_value(&template).expect("serializes");
    json["current"] = serde_json::json!(99);
    let saved: LayoutRing = serde_json::from_value(json).expect("deserializes");
    assert_eq!(template.restore(&saved), template);
}

#[test]
fn ring_round_trips_through_json() {
    let mut ring = LayoutRing::standard(20);
    ring.handle(
        LayoutMessage::Shrink,
        LayoutContext {
            focus: Some(0),
            count: 2,
        },
    );
    ring.next();
    let json = serde_json::to_string(&ring).expect("serializes");
    let read: LayoutRing = serde_json::from_str(&json).expect("deserializes");
    assert_eq!(read, ring);
}

fn corner() -> Corner {
    Corner::new(Ratio::new(3, 100), Ratio::new(2, 3), Ratio::new(2, 3))
}

const QHD: Rect = Rect {
    x: 0,
    y: 0,
    w: 2560,
    h: 1440,
};

#[test]
fn corner_with_few_windows_is_a_master_column() {
    assert_eq!(corner().arrange(QHD, 1), [QHD]);
    // Up to two others: all in the right column, the master full height.
    assert_eq!(
        corner().arrange(QHD, 3),
        [
            Rect::new(0, 0, 1706, 1440),
            Rect::new(1706, 0, 854, 720),
            Rect::new(1706, 720, 854, 720),
        ]
    );
}

#[test]
fn corner_puts_the_others_in_an_l_around_the_master() {
    // Four others: three down the right edge, one below the master.
    assert_eq!(
        corner().arrange(QHD, 5),
        [
            Rect::new(0, 0, 1706, 960),
            Rect::new(1706, 0, 854, 480),
            Rect::new(1706, 480, 854, 480),
            Rect::new(1706, 960, 854, 480),
            Rect::new(0, 960, 1706, 480),
        ]
    );
    // Five others: the 3 x 3 grid with a 2 x 2 master.
    let six = corner().arrange(QHD, 6);
    assert_eq!(six[0], Rect::new(0, 0, 1706, 960));
    assert_eq!(
        &six[4..],
        [Rect::new(0, 960, 853, 480), Rect::new(853, 960, 853, 480)]
    );
}

#[test]
fn corner_tiles_the_whole_area() {
    for n in 1..=12 {
        let rects = corner().arrange(QHD, n);
        assert_eq!(rects.len(), n);
        let area: i64 = rects.iter().map(|r| i64::from(r.w) * i64::from(r.h)).sum();
        assert_eq!(area, i64::from(QHD.w) * i64::from(QHD.h), "{n} windows");
        for (i, a) in rects.iter().enumerate() {
            for b in &rects[i + 1..] {
                let overlap =
                    a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h;
                assert!(!overlap, "{n} windows: {a:?} overlaps {b:?}");
            }
        }
    }
}

#[test]
fn corner_resizes_the_master() {
    let mut layout = Layout::Corner(corner());
    let ctx = LayoutContext {
        focus: Some(0),
        count: 5,
    };
    assert!(layout.handle(LayoutMessage::Expand, ctx));
    assert!(layout.handle(LayoutMessage::MirrorExpand, ctx));
    let master = layout.arrange(QHD, 5, 0).rects[0].1;
    // (2/3 + 3/100) of 2560 x 1440.
    assert_eq!(master, Rect::new(0, 0, 1783, 1003));
    assert!(!layout.handle(LayoutMessage::IncMasterN(1), ctx));
    // Nothing to resize without a tiled focus.
    let none = LayoutContext {
        focus: None,
        count: 0,
    };
    assert!(!layout.handle(LayoutMessage::Shrink, none));
}
