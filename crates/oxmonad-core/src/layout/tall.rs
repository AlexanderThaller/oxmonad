//! `XMonad.Layout.ResizableTile.ResizableTall`: master/stack tiling with a
//! resizable master column and per-slot height fractions.

use super::{
    LayoutContext,
    LayoutMessage,
    split::split_horizontally_by,
};
use crate::geometry::{
    Ratio,
    Rect,
};

/// xmonad-contrib's `ResizableTall`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResizableTall {
    /// Number of windows in the master column.
    pub nmaster: usize,
    /// Step for `Shrink`/`Expand`/`MirrorShrink`/`MirrorExpand`.
    pub delta: Ratio,
    /// Width of the master column as a fraction of the area.
    pub frac: Ratio,
    /// Per-slot factors multiplying the height a slot would get when the
    /// column is divided equally, top to bottom; missing entries are 1.
    pub slaves: Vec<Ratio>,
}

impl ResizableTall {
    /// `ResizableTall nmaster delta frac []`.
    #[must_use]
    pub fn new(nmaster: usize, delta: Ratio, frac: Ratio) -> Self {
        Self {
            nmaster,
            delta,
            frac,
            slaves: Vec::new(),
        }
    }

    /// The slot factor at `index` (`mfrac ++ repeat 1`).
    fn slave(&self, index: usize) -> Ratio {
        self.slaves.get(index).copied().unwrap_or(Ratio::ONE)
    }

    /// Ports `tile`: rectangles for `n` windows in stack order.
    #[must_use]
    pub fn arrange(&self, area: Rect, n: usize) -> Vec<Rect> {
        if n == 0 {
            return Vec::new();
        }
        if n <= self.nmaster || self.nmaster == 0 {
            return self.split_vertically(0, n, area);
        }
        let (master, stack) = split_horizontally_by(self.frac, area);
        let mut out = self.split_vertically(0, self.nmaster, master);
        out.extend(self.split_vertically(self.nmaster, n - self.nmaster, stack));
        out
    }

    /// `ResizableTile`'s `splitVertically`, using the slot factors starting
    /// at `first`.
    ///
    /// `smallh = min sh (floor (fromIntegral (sh `div` n) * f))`; the
    /// Haskell result is a `Word32`, so a negative floor wraps to a huge
    /// value and the `min` yields the whole remaining height.
    fn split_vertically(&self, first: usize, n: usize, area: Rect) -> Vec<Rect> {
        let mut out = Vec::with_capacity(n.max(1));
        let mut rest = area;
        let mut left = n;
        let mut slot = first;
        while left >= 2 {
            let count = i32::try_from(left).unwrap_or(i32::MAX);
            let scaled = self.slave(slot).floor_mul(rest.h / count);
            let small = if scaled < 0 {
                rest.h
            } else {
                scaled.min(rest.h)
            };
            out.push(Rect::new(rest.x, rest.y, rest.w, small));
            rest = Rect::new(rest.x, rest.y + small, rest.w, rest.h - small);
            left -= 1;
            slot += 1;
        }
        out.push(rest);
        out
    }

    /// Ports `handleMessage`. Every message is ignored when there is no
    /// tiled focus (an empty workspace or a floating focused window).
    pub fn handle(&mut self, msg: LayoutMessage, ctx: LayoutContext) -> bool {
        let Some(focus) = ctx.focus else {
            return false;
        };
        match msg {
            LayoutMessage::Shrink => self.frac = (self.frac - self.delta).max(Ratio::ZERO),
            LayoutMessage::Expand => self.frac = (self.frac + self.delta).min(Ratio::ONE),
            LayoutMessage::MirrorShrink => self.mresize(focus, ctx.count, self.delta),
            LayoutMessage::MirrorExpand => self.mresize(focus, ctx.count, -self.delta),
            LayoutMessage::IncMasterN(d) => self.nmaster = inc_master(self.nmaster, d),
        }
        true
    }

    /// `mresize'`: adjusts the factor of the focused slot, or of the slot
    /// above when the focused window is the last of its column.
    fn mresize(&mut self, focus: usize, total: usize, d: Ratio) {
        let n = to_i64(focus);
        let pos = if n == to_i64(self.nmaster) - 1 || n == to_i64(total) - 1 {
            n - 1
        } else {
            n
        };
        let mut slaves: Vec<Ratio> = (0..total).map(|i| self.slave(i)).collect();
        if let Ok(pos) = usize::try_from(pos)
            && let Some(slot) = slaves.get_mut(pos)
        {
            *slot = *slot + d;
        }
        self.slaves = slaves;
    }
}

/// `IncMasterN`: `max 0 (nmaster + d)`.
pub(super) fn inc_master(nmaster: usize, d: i32) -> usize {
    usize::try_from((to_i64(nmaster) + i64::from(d)).max(0)).unwrap_or(0)
}

fn to_i64(n: usize) -> i64 {
    i64::try_from(n).unwrap_or(i64::MAX)
}
