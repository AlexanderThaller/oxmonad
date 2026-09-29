//! `XMonad.Layout.Grid.Grid`: a grid striving for a 16:9 cell aspect.

use crate::geometry::{
    Rect,
    round_half_even,
};

/// xmonad-contrib's `defaultRatio` for a bare `Grid`.
const DEFAULT_RATIO: f64 = 16.0 / 9.0;

/// Ports `Grid.arrange`: choose a column count from the aspect ratio, fill
/// columns top to bottom and give the rightmost columns one extra window.
pub(super) fn arrange(area: Rect, n: usize) -> Vec<Rect> {
    if n == 0 {
        return Vec::new();
    }
    let nwins = i32::try_from(n).unwrap_or(i32::MAX);
    // ncols = max 1 . min nwins . round . sqrt $ nwins * rw / (rh * ratio)
    let ideal = (f64::from(nwins) * f64::from(area.w) / (f64::from(area.h) * DEFAULT_RATIO)).sqrt();
    let ncols = i32::try_from(round_half_even(ideal).clamp(1, i64::from(nwins))).unwrap_or(1);
    let mincs = (nwins / ncols).max(1);
    let extrs = nwins - ncols * mincs;
    let xcoords = chop(ncols, area.w);
    let ycoords = chop(mincs, area.h);
    let ycoords_extra = chop(mincs + 1, area.h);
    let split = usize::try_from(ncols - extrs)
        .unwrap_or(0)
        .min(xcoords.len());
    let (xbase, xext) = xcoords.split_at(split);
    let mut out = Vec::with_capacity(n);
    combine(&mut out, area, &ycoords, xbase);
    combine(&mut out, area, &ycoords_extra, xext);
    out
}

/// Column-major cells: for each column `(x, w)` one rectangle per row.
fn combine(out: &mut Vec<Rect>, area: Rect, ys: &[(i32, i32)], xs: &[(i32, i32)]) {
    for &(x, w) in xs {
        for &(y, h) in ys {
            out.push(Rect::new(area.x + x, area.y + y, w, h));
        }
    }
}

/// `chop n m`: `n` cells of `m` pixels, `k = m div n` each, the first one
/// absorbing the remainder: `(0, m - k(n-1))` then `(m - (n-i)k, k)`.
fn chop(n: i32, m: i32) -> Vec<(i32, i32)> {
    let n = n.max(1);
    let k = m / n;
    let mut out = vec![(0, m - k * (n - 1))];
    out.extend((1..n).map(|i| (m - (n - i) * k, k)));
    out
}
