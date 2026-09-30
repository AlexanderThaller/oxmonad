//! `Corner`: the master in the top-left corner, the other windows in an L
//! around it — a column down the right edge (including the bottom-right
//! corner) and a row below the master.

use super::{
    LayoutMessage,
    split::split_vertically,
};
use crate::geometry::{
    Ratio,
    Rect,
};
use serde::{
    Deserialize,
    Serialize,
};

/// The corner layout's state: the master's share of the width and height.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Corner {
    /// Step for `Shrink`/`Expand`/`MirrorShrink`/`MirrorExpand`.
    pub delta: Ratio,
    /// Width of the master as a fraction of the area.
    pub width: Ratio,
    /// Height of the master as a fraction of the area.
    pub height: Ratio,
}

impl Corner {
    /// A corner layout whose master covers `width` × `height` of the area.
    #[must_use]
    pub fn new(delta: Ratio, width: Ratio, height: Ratio) -> Self {
        Self {
            delta,
            width,
            height,
        }
    }

    /// Rectangles for `n` windows in stack order: the master, then the
    /// right column top to bottom, then the bottom row left to right.
    ///
    /// The `k = n - 1` other windows are shared 3 : 2 between the column
    /// and the row, like the cells around a 2 × 2 master in a 3 × 3 grid:
    /// `row = floor(2k / 5)`, so five windows form exactly that grid.
    /// Without a row the master takes the full height, without any other
    /// window the whole area.
    #[must_use]
    pub fn arrange(&self, area: Rect, n: usize) -> Vec<Rect> {
        if n == 0 {
            return Vec::new();
        }
        if n == 1 {
            return vec![area];
        }
        let others = n - 1;
        let master_w = self.width.floor_mul(area.w);
        let master_h = self.height.floor_mul(area.h);
        let row = self.row_count(others);
        let column = others - row;

        let master_h = if row == 0 { area.h } else { master_h };
        let mut out = Vec::with_capacity(n);
        out.push(Rect::new(area.x, area.y, master_w, master_h));
        out.extend(split_vertically(
            column,
            Rect::new(area.x + master_w, area.y, area.w - master_w, area.h),
        ));
        if row > 0 {
            let strip = Rect::new(area.x, area.y + master_h, master_w, area.h - master_h);
            out.extend(
                split_vertically(row, strip.transposed())
                    .into_iter()
                    .map(|r| r.transposed()),
            );
        }
        out
    }

    /// How many of `others` windows go into the bottom row (see
    /// [`Self::arrange`]); none when the master is full height.
    fn row_count(&self, others: usize) -> usize {
        if self.height >= Ratio::ONE {
            return 0;
        }
        (others * 2 / 5).min(others - 1)
    }

    /// `Shrink`/`Expand` change the master's width, `MirrorShrink`/
    /// `MirrorExpand` its height (so Mod+Up makes it taller); the master
    /// count is fixed at one.
    pub fn handle(&mut self, msg: LayoutMessage) -> bool {
        let step = |value: Ratio, by: Ratio| (value + by).clamp(Ratio::ZERO, Ratio::ONE);
        match msg {
            LayoutMessage::Shrink => self.width = step(self.width, -self.delta),
            LayoutMessage::Expand => self.width = step(self.width, self.delta),
            LayoutMessage::MirrorShrink => self.height = step(self.height, -self.delta),
            LayoutMessage::MirrorExpand => self.height = step(self.height, self.delta),
            LayoutMessage::IncMasterN(_) => return false,
        }
        true
    }
}
