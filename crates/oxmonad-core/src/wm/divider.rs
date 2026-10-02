//! Divider lines between the virtual screens of a split output.
//!
//! [`divide`] is pure geometry: given the tiling areas of one output's
//! screens (usable area minus the bar), it finds every edge two of them
//! share and centres a divider strip on it. The screens give that space
//! up, plus some padding on either side, so no tiled window (or its
//! border) sits under or against a divider.

use crate::geometry::Rect;

/// The dividers of one output and its screens' tiling areas without them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Division {
    /// The divider strips, one per shared edge, in pair order.
    pub dividers: Vec<Rect>,
    /// The screens' areas, in the order given, minus the dividers.
    pub areas: Vec<Rect>,
}

/// How much a screen's area gives up on each side.
#[derive(Debug, Clone, Copy, Default)]
struct Insets {
    left: i32,
    right: i32,
    top: i32,
    bottom: i32,
}

/// Dividers `width` wide on every edge that two of `areas` share, and the
/// areas shrunk so they keep `padding` away from them.
///
/// Two areas share an edge when one ends exactly where the other starts
/// (along x for side-by-side screens, along y for stacked ones) and they
/// overlap along that edge. The divider is centred on the edge and spans
/// that overlap: the screen on the left (or top) gives up `floor(width /
/// 2)`, the one on the right (or below) `ceil(width / 2)`, so an odd
/// width puts its extra pixel on the right (bottom) side of the edge. Each
/// screen also gives up `padding` between its area and the divider, which
/// stays empty. A screen next to several screens on one side gives up the
/// space once. Outer edges get nothing; a `width` of `0` (or less) changes
/// nothing, padding included.
pub(super) fn divide(areas: &[Rect], width: i32, padding: i32) -> Division {
    let mut insets = vec![Insets::default(); areas.len()];
    let mut dividers = Vec::new();
    if width > 0 {
        let padding = padding.max(0);
        let before = width / 2;
        let after = width - before;
        let (give_before, give_after) = (before + padding, after + padding);
        for (i, a) in areas.iter().enumerate() {
            for (j, b) in areas.iter().enumerate().skip(i + 1) {
                if a.w <= 0 || a.h <= 0 || b.w <= 0 || b.h <= 0 {
                    continue;
                }
                // Side by side: `(left, right)`.
                let beside = if a.x + a.w == b.x {
                    Some((i, j))
                } else if b.x + b.w == a.x {
                    Some((j, i))
                } else {
                    None
                };
                if let Some((l, r)) = beside {
                    let edge = areas[r].x;
                    let (top, bottom) = (a.y.max(b.y), (a.y + a.h).min(b.y + b.h));
                    if bottom > top {
                        dividers.push(Rect::new(edge - before, top, width, bottom - top));
                        insets[l].right = insets[l].right.max(give_before);
                        insets[r].left = insets[r].left.max(give_after);
                    }
                    continue;
                }
                // Stacked: `upper` is the top one.
                let (upper, lower) = if a.y + a.h == b.y {
                    (i, j)
                } else if b.y + b.h == a.y {
                    (j, i)
                } else {
                    continue;
                };
                let edge = areas[lower].y;
                let (start, end) = (a.x.max(b.x), (a.x + a.w).min(b.x + b.w));
                if end > start {
                    dividers.push(Rect::new(start, edge - before, end - start, width));
                    insets[upper].bottom = insets[upper].bottom.max(give_before);
                    insets[lower].top = insets[lower].top.max(give_after);
                }
            }
        }
    }
    let areas = areas
        .iter()
        .zip(insets)
        .map(|(area, inset)| {
            Rect::new(
                area.x + inset.left,
                area.y + inset.top,
                (area.w - inset.left - inset.right).max(0),
                (area.h - inset.top - inset.bottom).max(0),
            )
        })
        .collect();
    Division { dividers, areas }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NO_DIVIDERS: [Rect; 0] = [];

    #[test]
    fn two_columns_share_one_divider() {
        let areas = [Rect::new(0, 24, 960, 1056), Rect::new(960, 24, 960, 1056)];
        let d = divide(&areas, 3, 0);
        assert_eq!(d.dividers, [Rect::new(959, 24, 3, 1056)]);
        assert_eq!(
            d.areas,
            [Rect::new(0, 24, 959, 1056), Rect::new(962, 24, 958, 1056)]
        );
        // Even widths split evenly.
        let d = divide(&areas, 4, 0);
        assert_eq!(d.dividers, [Rect::new(958, 24, 4, 1056)]);
        assert_eq!(
            d.areas,
            [Rect::new(0, 24, 958, 1056), Rect::new(962, 24, 958, 1056)]
        );
    }

    #[test]
    fn padding_keeps_the_areas_away_from_the_divider() {
        let areas = [Rect::new(0, 24, 960, 1056), Rect::new(960, 24, 960, 1056)];
        let d = divide(&areas, 3, 2);
        // The line does not move; each side gives up two more pixels.
        assert_eq!(d.dividers, [Rect::new(959, 24, 3, 1056)]);
        assert_eq!(
            d.areas,
            [Rect::new(0, 24, 957, 1056), Rect::new(964, 24, 956, 1056)]
        );
        // Stacked screens get it above and below.
        let areas = [Rect::new(0, 0, 1920, 540), Rect::new(0, 540, 1920, 540)];
        let d = divide(&areas, 2, 3);
        assert_eq!(d.dividers, [Rect::new(0, 539, 1920, 2)]);
        assert_eq!(
            d.areas,
            [Rect::new(0, 0, 1920, 536), Rect::new(0, 544, 1920, 536)]
        );
        // No divider, no padding.
        let areas = [Rect::new(0, 0, 960, 1080), Rect::new(960, 0, 960, 1080)];
        assert_eq!(divide(&areas, 0, 5).areas, areas);
    }

    #[test]
    fn zero_width_changes_nothing() {
        let areas = [Rect::new(0, 0, 960, 1080), Rect::new(960, 0, 960, 1080)];
        let d = divide(&areas, 0, 0);
        assert_eq!(d.dividers, NO_DIVIDERS);
        assert_eq!(d.areas, areas);
        assert_eq!(divide(&areas, -2, 0).areas, areas);
    }

    #[test]
    fn a_single_area_gets_no_divider() {
        let areas = [Rect::new(0, 0, 1920, 1080)];
        let d = divide(&areas, 3, 0);
        assert_eq!(d.dividers, NO_DIVIDERS);
        assert_eq!(d.areas, areas);
    }

    #[test]
    fn a_middle_column_gives_up_space_on_both_sides() {
        let areas = [
            Rect::new(0, 0, 640, 1080),
            Rect::new(640, 0, 640, 1080),
            Rect::new(1280, 0, 640, 1080),
        ];
        let d = divide(&areas, 3, 0);
        assert_eq!(
            d.dividers,
            [Rect::new(639, 0, 3, 1080), Rect::new(1279, 0, 3, 1080)]
        );
        assert_eq!(
            d.areas,
            [
                Rect::new(0, 0, 639, 1080),
                Rect::new(642, 0, 637, 1080),
                Rect::new(1282, 0, 638, 1080),
            ]
        );
    }

    #[test]
    fn stacked_areas_get_a_horizontal_divider() {
        let areas = [Rect::new(0, 0, 1920, 540), Rect::new(0, 540, 1920, 540)];
        let d = divide(&areas, 5, 0);
        assert_eq!(d.dividers, [Rect::new(0, 538, 1920, 5)]);
        assert_eq!(
            d.areas,
            [Rect::new(0, 0, 1920, 538), Rect::new(0, 543, 1920, 537)]
        );
    }

    #[test]
    fn a_divider_spans_only_the_overlap() {
        // A left column next to two stacked right screens, in any order.
        let areas = [
            Rect::new(960, 540, 960, 540),
            Rect::new(0, 0, 960, 1080),
            Rect::new(960, 0, 960, 540),
        ];
        let d = divide(&areas, 2, 0);
        assert_eq!(
            d.dividers,
            [
                Rect::new(959, 540, 2, 540),
                Rect::new(960, 539, 960, 2),
                Rect::new(959, 0, 2, 540),
            ]
        );
        assert_eq!(
            d.areas,
            [
                Rect::new(961, 541, 959, 539),
                Rect::new(0, 0, 959, 1080),
                Rect::new(961, 0, 959, 539),
            ]
        );
    }

    #[test]
    fn areas_that_only_touch_at_a_corner_or_not_at_all_get_none() {
        let corner = [Rect::new(0, 0, 100, 100), Rect::new(100, 100, 100, 100)];
        assert_eq!(divide(&corner, 3, 0).dividers, NO_DIVIDERS);
        let apart = [Rect::new(0, 0, 100, 100), Rect::new(101, 0, 100, 100)];
        assert_eq!(divide(&apart, 3, 0).dividers, NO_DIVIDERS);
        let empty = [Rect::new(0, 0, 100, 0), Rect::new(100, 0, 100, 0)];
        assert_eq!(divide(&empty, 3, 0).dividers, NO_DIVIDERS);
    }
}
