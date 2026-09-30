//! `XMonad.Layout.ThreeColumns.ThreeCol`: a master column on the left and
//! the stack split over two further columns.

use super::{
    LayoutMessage,
    split::{
        split_horizontally_by,
        split_vertically,
    },
    tall::inc_master,
};
use crate::geometry::{
    Ratio,
    Rect,
};
use serde::{
    Deserialize,
    Serialize,
};

/// xmonad-contrib's `ThreeCol` (not `ThreeColMid`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreeColumns {
    /// Number of windows in the master column.
    pub nmaster: usize,
    /// Step for `Shrink`/`Expand`.
    pub delta: Ratio,
    /// Master column width fraction; negative values are interpreted as in
    /// xmonad (`1 + 2f` with three columns, `1 + f` with two).
    pub frac: Ratio,
}

impl ThreeColumns {
    /// `ThreeCol nmaster delta frac`.
    #[must_use]
    pub const fn new(nmaster: usize, delta: Ratio, frac: Ratio) -> Self {
        Self {
            nmaster,
            delta,
            frac,
        }
    }

    /// Ports `tile3 False`: rectangles for `n` windows in stack order.
    #[must_use]
    pub fn arrange(&self, area: Rect, n: usize) -> Vec<Rect> {
        if n == 0 {
            return Vec::new();
        }
        let nmaster = self.nmaster;
        let f = self.frac;
        if n <= nmaster || nmaster == 0 {
            return split_vertically(n, area);
        }
        if n <= nmaster + 1 {
            let f = if f.is_negative() { Ratio::ONE + f } else { f };
            let (s1, s2) = split_horizontally_by(f, area);
            let mut out = split_vertically(nmaster, s1);
            out.extend(split_vertically(n - nmaster, s2));
            return out;
        }
        let f = if f.is_negative() {
            Ratio::ONE + Ratio::from_int(2) * f
        } else {
            f
        };
        let (r1, r2, r3) = split3_horizontally_by(f, area);
        let nslave = n - nmaster;
        let nslave1 = nslave.div_ceil(2);
        let nslave2 = n - nmaster - nslave1;
        let mut out = split_vertically(nmaster, r1);
        out.extend(split_vertically(nslave1, r2));
        out.extend(split_vertically(nslave2, r3));
        out
    }

    /// Ports `handleMessage`; `MirrorShrink`/`MirrorExpand` are ignored.
    pub fn handle(&mut self, msg: LayoutMessage) -> bool {
        match msg {
            LayoutMessage::Shrink => {
                self.frac = (self.frac - self.delta).max(Ratio::new(-1, 2));
            }
            LayoutMessage::Expand => self.frac = (self.frac + self.delta).min(Ratio::ONE),
            LayoutMessage::IncMasterN(d) => self.nmaster = inc_master(self.nmaster, d),
            LayoutMessage::MirrorShrink | LayoutMessage::MirrorExpand => return false,
        }
        true
    }
}

/// `split3HorizontallyBy False`: `r1w = ceiling (w * f)`,
/// `r2w = ceiling ((w - r1w) / 2)`, `r3w` the remainder.
fn split3_horizontally_by(f: Ratio, area: Rect) -> (Rect, Rect, Rect) {
    let r1w = f.ceil_mul(area.w);
    let r2w = Ratio::new(1, 2).ceil_mul(area.w - r1w);
    let r3w = area.w - r1w - r2w;
    (
        Rect::new(area.x, area.y, r1w, area.h),
        Rect::new(area.x + r1w, area.y, r2w, area.h),
        Rect::new(area.x + r1w + r2w, area.y, r3w, area.h),
    )
}
