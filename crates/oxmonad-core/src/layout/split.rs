//! Rectangle splitting helpers from xmonad's `XMonad.Layout`.

use crate::geometry::{
    Ratio,
    Rect,
};

/// xmonad's core `splitVertically`: `n` stacked rows, each the remaining
/// height divided by the remaining count, so the last row absorbs the
/// rounding remainder.
pub(super) fn split_vertically(n: usize, area: Rect) -> Vec<Rect> {
    let mut out = Vec::with_capacity(n.max(1));
    let mut rest = area;
    let mut left = n;
    while left >= 2 {
        let count = i32::try_from(left).unwrap_or(i32::MAX);
        let small = rest.h / count;
        out.push(Rect::new(rest.x, rest.y, rest.w, small));
        rest = Rect::new(rest.x, rest.y + small, rest.w, rest.h - small);
        left -= 1;
    }
    out.push(rest);
    out
}

/// xmonad's `splitHorizontallyBy`: a left part `floor (w * f)` wide and the
/// remainder.
pub(super) fn split_horizontally_by(f: Ratio, area: Rect) -> (Rect, Rect) {
    let left = f.floor_mul(area.w);
    (
        Rect::new(area.x, area.y, left, area.h),
        Rect::new(area.x + left, area.y, area.w - left, area.h),
    )
}
