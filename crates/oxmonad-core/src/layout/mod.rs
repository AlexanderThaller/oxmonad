//! Layouts and the per-workspace layout ring.
//!
//! Each [`Layout`] is a pure function from an area and a window count to
//! rectangles, plus state changed by [`LayoutMessage`]s. The tilers are
//! ported from the xmonad / xmonad-contrib Haskell sources in `resources/`,
//! including their integer rounding. A [`LayoutRing`] is xmonad's
//! `l1 ||| l2 ||| …` choice: every entry keeps its own state and
//! [`LayoutRing::next`] rotates between them.

mod grid;
mod split;
mod tall;
mod three_col;

#[cfg(test)]
mod tests;

pub use tall::ResizableTall;
pub use three_col::ThreeCol;

use crate::geometry::{
    Ratio,
    Rect,
};
use serde::{
    Deserialize,
    Serialize,
};

/// Messages a layout may react to (`sendMessage`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LayoutMessage {
    /// Shrink the master area.
    Shrink,
    /// Expand the master area.
    Expand,
    /// `ResizableTall`'s `MirrorShrink`: grow the focused slot's factor.
    MirrorShrink,
    /// `ResizableTall`'s `MirrorExpand`: shrink the focused slot's factor.
    MirrorExpand,
    /// Change the number of master windows by the given amount.
    IncMasterN(i32),
}

/// Tiled-stack context for messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LayoutContext {
    /// Index of the focused window within the tiled (non-floating) windows,
    /// or `None` when the workspace has no tiled windows or the focused
    /// window is floating.
    pub focus: Option<usize>,
    /// Number of tiled windows.
    pub count: usize,
}

/// Geometry of a tab strip (`tabbed` with at least two windows).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TabGeometry {
    /// The whole strip.
    pub strip: Rect,
    /// One rectangle per tiled window, in stack order.
    pub tabs: Vec<Rect>,
}

/// The result of arranging a workspace's tiled windows.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LayoutOutput {
    /// `(index into the tiled windows, rectangle)` pairs; windows that are
    /// not listed are hidden.
    pub rects: Vec<(usize, Rect)>,
    /// The tab strip, when the layout shows one.
    pub tabs: Option<TabGeometry>,
}

impl LayoutOutput {
    fn all(rects: Vec<Rect>) -> Self {
        Self {
            rects: rects.into_iter().enumerate().collect(),
            tabs: None,
        }
    }

    fn map(self, f: impl Fn(Rect) -> Rect) -> Self {
        Self {
            rects: self.rects.into_iter().map(|(i, r)| (i, f(r))).collect(),
            tabs: self.tabs.map(|tabs| TabGeometry {
                strip: f(tabs.strip),
                tabs: tabs.tabs.into_iter().map(&f).collect(),
            }),
        }
    }
}

/// A layout, possibly wrapped in modifiers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Layout {
    /// `ResizableTall`.
    Tall(ResizableTall),
    /// `ThreeCol`.
    ThreeCol(ThreeCol),
    /// `Grid` (aspect ratio 16/9).
    Grid,
    /// `Full`: only the focused window, covering the area.
    Full,
    /// `tabbed shrinkText theme`: the focused window below a strip of tabs
    /// (the strip is only shown with two or more windows).
    Tabbed {
        /// Height of the tab strip (the theme's `decoHeight`).
        tab_height: i32,
    },
    /// `Mirror`: the inner layout rotated by 90 degrees.
    Mirror(Box<Layout>),
    /// `reflectHoriz`: the inner layout flipped left to right.
    ReflectX(Box<Layout>),
    /// `noBorders`: the inner layout without window borders.
    NoBorders(Box<Layout>),
}

impl Layout {
    /// Arranges `n` tiled windows in `area`; `focus` is the focused
    /// window's index (`focus < n`).
    #[must_use]
    pub fn arrange(&self, area: Rect, n: usize, focus: usize) -> LayoutOutput {
        if n == 0 {
            return LayoutOutput::default();
        }
        let focus = focus.min(n - 1);
        match self {
            Self::Tall(tall) => LayoutOutput::all(tall.arrange(area, n)),
            Self::ThreeCol(cols) => LayoutOutput::all(cols.arrange(area, n)),
            Self::Grid => LayoutOutput::all(grid::arrange(area, n)),
            Self::Full => LayoutOutput {
                rects: vec![(focus, area)],
                tabs: None,
            },
            Self::Tabbed { tab_height } => tabbed(area, n, focus, *tab_height),
            Self::Mirror(inner) => inner
                .arrange(area.transposed(), n, focus)
                .map(|r| r.transposed()),
            Self::ReflectX(inner) => inner
                .arrange(area, n, focus)
                .map(|r| Rect::new(2 * area.x + area.w - r.x - r.w, r.y, r.w, r.h)),
            Self::NoBorders(inner) => inner.arrange(area, n, focus),
        }
    }

    /// Handles a message; returns whether the layout accepted it.
    pub fn handle(&mut self, msg: LayoutMessage, ctx: LayoutContext) -> bool {
        match self {
            Self::Tall(tall) => tall.handle(msg, ctx),
            Self::ThreeCol(cols) => cols.handle(msg),
            Self::Grid | Self::Full | Self::Tabbed { .. } => false,
            Self::Mirror(inner) | Self::ReflectX(inner) | Self::NoBorders(inner) => {
                inner.handle(msg, ctx)
            }
        }
    }

    /// The name shown in the bar: plain words for where the master
    /// window goes, instead of xmonad's modifier chains (see
    /// [`Self::description`]).
    #[must_use]
    pub fn name(&self) -> String {
        match self {
            Self::Tall(_) => "Tall".to_owned(),
            Self::ThreeCol(_) => "Three Columns".to_owned(),
            Self::Grid => "Grid".to_owned(),
            Self::Full => "Full".to_owned(),
            Self::Tabbed { .. } => "Tabs".to_owned(),
            Self::Mirror(inner) => match **inner {
                Self::Tall(_) => "Wide".to_owned(),
                Self::Grid => "Grid Rows".to_owned(),
                Self::ThreeCol(_)
                | Self::Full
                | Self::Tabbed { .. }
                | Self::Mirror(_)
                | Self::ReflectX(_)
                | Self::NoBorders(_) => format!("{} Mirrored", inner.name()),
            },
            Self::ReflectX(inner) => format!("{} Right", inner.name()),
            Self::NoBorders(inner) => inner.name(),
        }
    }

    /// xmonad's `description`. Not shown to users; it identifies a layout
    /// in restart snapshots, so it must stay stable.
    #[must_use]
    pub fn description(&self) -> String {
        match self {
            Self::Tall(_) => "ResizableTall".to_owned(),
            Self::ThreeCol(_) => "ThreeCol".to_owned(),
            Self::Grid => "Grid".to_owned(),
            Self::Full => "Full".to_owned(),
            Self::Tabbed { .. } => "Tabbed Simplest".to_owned(),
            Self::Mirror(inner) => format!("Mirror {}", inner.description()),
            Self::ReflectX(inner) => modified("ReflectX", &inner.description()),
            Self::NoBorders(inner) => modified("", &inner.description()),
        }
    }

    /// Whether windows arranged by this layout get borders.
    #[must_use]
    pub fn has_borders(&self) -> bool {
        match self {
            Self::NoBorders(_) => false,
            Self::Mirror(inner) | Self::ReflectX(inner) => inner.has_borders(),
            Self::Tall(_) | Self::ThreeCol(_) | Self::Grid | Self::Full | Self::Tabbed { .. } => {
                true
            }
        }
    }
}

/// `LayoutModifier`'s description join: a space only between two
/// non-empty parts.
fn modified(modifier: &str, inner: &str) -> String {
    match (modifier.is_empty(), inner.is_empty()) {
        (true, _) => inner.to_owned(),
        (false, true) => modifier.to_owned(),
        (false, false) => format!("{modifier} {inner}"),
    }
}

/// `addTabs WhenPlural U Simplest`: the focused window below the strip, tab
/// `i` spanning `x + w*i/n .. x + w*(i+1)/n` (Tabbed.hs `loc`/`esize`).
fn tabbed(area: Rect, n: usize, focus: usize, tab_height: i32) -> LayoutOutput {
    if n < 2 {
        return LayoutOutput {
            rects: vec![(focus, area)],
            tabs: None,
        };
    }
    let count = i64::try_from(n).unwrap_or(i64::MAX);
    let loc = |i: i64| area.x + crate::geometry::clamp_i32(i64::from(area.w) * i / count);
    let tabs = (0..count)
        .map(|i| Rect::new(loc(i), area.y, loc(i + 1) - loc(i), tab_height))
        .collect();
    LayoutOutput {
        rects: vec![(
            focus,
            Rect::new(area.x, area.y + tab_height, area.w, area.h - tab_height),
        )],
        tabs: Some(TabGeometry {
            strip: Rect::new(area.x, area.y, area.w, tab_height),
            tabs,
        }),
    }
}

/// A workspace's ring of layouts (`l1 ||| l2 ||| …`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayoutRing {
    layouts: Vec<Layout>,
    current: usize,
}

impl LayoutRing {
    /// A ring of `layouts` starting at the first; an empty list yields
    /// `[Full]`.
    #[must_use]
    pub fn new(layouts: Vec<Layout>) -> Self {
        let layouts = if layouts.is_empty() {
            vec![Layout::Full]
        } else {
            layouts
        };
        Self {
            layouts,
            current: 0,
        }
    }

    /// The reference config's `myLayout`: `reflectHoriz tiled ||| tiled |||
    /// Mirror tiled ||| noBorders Full ||| tabbed ||| Mirror Grid ||| Grid
    /// ||| ThreeCol ||| reflectHoriz ThreeCol` with
    /// `tiled = ResizableTall 1 (3/100) (3/5) []` and
    /// `ThreeCol 1 (3/100) (1/2)`.
    #[must_use]
    pub fn xmonad_default(tab_height: i32) -> Self {
        let tiled = || Layout::Tall(ResizableTall::new(1, Ratio::new(3, 100), Ratio::new(3, 5)));
        let three = || Layout::ThreeCol(ThreeCol::new(1, Ratio::new(3, 100), Ratio::new(1, 2)));
        Self::new(vec![
            Layout::ReflectX(Box::new(tiled())),
            tiled(),
            Layout::Mirror(Box::new(tiled())),
            Layout::NoBorders(Box::new(Layout::Full)),
            Layout::Tabbed { tab_height },
            Layout::Mirror(Box::new(Layout::Grid)),
            Layout::Grid,
            three(),
            Layout::ReflectX(Box::new(three())),
        ])
    }

    /// The ring to use for a workspace whose ring was `saved` (e.g. before
    /// a restart), with `self` being the configured ring.
    ///
    /// If `saved` has the same layouts as `self` (by description), it is
    /// kept with every entry's state; only the `tabbed` entries take the
    /// configured tab height. Otherwise `self` is used, switched to the
    /// layout `saved` had active if the configured ring has one with that
    /// description.
    #[must_use]
    pub fn restore(&self, saved: &Self) -> Self {
        let compatible = saved.current < saved.layouts.len()
            && saved.layouts.len() == self.layouts.len()
            && saved
                .layouts
                .iter()
                .zip(&self.layouts)
                .all(|(a, b)| a.description() == b.description());
        if compatible {
            let layouts = saved
                .layouts
                .iter()
                .zip(&self.layouts)
                .map(|(saved, configured)| {
                    if matches!(configured, Layout::Tabbed { .. }) {
                        configured.clone()
                    } else {
                        saved.clone()
                    }
                })
                .collect();
            return Self {
                layouts,
                current: saved.current,
            };
        }
        let mut ring = self.clone();
        if let Some(active) = saved.layouts.get(saved.current).map(Layout::description)
            && let Some(index) = ring.layouts.iter().position(|l| l.description() == active)
        {
            ring.current = index;
        }
        ring
    }

    /// The active layout.
    #[must_use]
    pub fn current(&self) -> &Layout {
        &self.layouts[self.current]
    }

    /// The active layout, mutably.
    pub fn current_mut(&mut self) -> &mut Layout {
        &mut self.layouts[self.current]
    }

    /// Index of the active layout.
    #[must_use]
    pub fn index(&self) -> usize {
        self.current
    }

    /// Number of layouts in the ring.
    #[must_use]
    pub fn len(&self) -> usize {
        self.layouts.len()
    }

    /// Always `false`: a ring holds at least one layout.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.layouts.is_empty()
    }

    /// `NextLayout`: rotates to the next entry, wrapping, keeping every
    /// entry's state.
    pub fn next(&mut self) {
        self.current = (self.current + 1) % self.layouts.len();
    }

    /// Arranges with the active layout (see [`Layout::arrange`]).
    #[must_use]
    pub fn arrange(&self, area: Rect, n: usize, focus: usize) -> LayoutOutput {
        self.current().arrange(area, n, focus)
    }

    /// Sends a message to the active layout (see [`Layout::handle`]).
    pub fn handle(&mut self, msg: LayoutMessage, ctx: LayoutContext) -> bool {
        self.current_mut().handle(msg, ctx)
    }

    /// Description of the active layout (see [`Layout::description`]).
    #[must_use]
    pub fn description(&self) -> String {
        self.current().description()
    }

    /// Display name of the active layout (see [`Layout::name`]).
    #[must_use]
    pub fn name(&self) -> String {
        self.current().name()
    }

    /// Whether the active layout draws borders.
    #[must_use]
    pub fn has_borders(&self) -> bool {
        self.current().has_borders()
    }
}
