//! Layouts and the per-workspace layout ring.
//!
//! Each [`Layout`] is a pure function from an area and a window count to
//! rectangles, plus state changed by [`LayoutMessage`]s. The tilers are
//! ported from the xmonad / xmonad-contrib Haskell sources in `resources/`,
//! including their integer rounding. A [`LayoutRing`] is xmonad's
//! `l1 ||| l2 ||| …` choice: every entry keeps its own state and
//! [`LayoutRing::next`] rotates between them.

mod corner;
mod grid;
mod split;
mod tall;
mod three_columns;

#[cfg(test)]
mod tests;

pub use corner::Corner;
pub use tall::Tall;
pub use three_columns::ThreeColumns;

use crate::geometry::{
    Ratio,
    Rect,
};
use serde::{
    Deserialize,
    Serialize,
};

/// The area a layout [preview](Layout::preview) is drawn in: a 16:9
/// screen, as in the layout pictures in `docs/layouts/`.
pub const PREVIEW_AREA: Rect = Rect::new(0, 0, 320, 180);

/// Windows in a layout [preview](Layout::preview); enough to show every
/// layout's stack.
pub const PREVIEW_WINDOWS: usize = 4;

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
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TabGeometry {
    /// The whole strip.
    pub strip: Rect,
    /// One rectangle per tiled window, in stack order.
    pub tabs: Vec<Rect>,
}

/// The result of arranging a workspace's tiled windows.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
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
    Tall(Tall),
    /// `ThreeCol`.
    ThreeColumns(ThreeColumns),
    /// The master in the top-left corner, the others in an L around it.
    Corner(Corner),
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
    Reflect(Box<Layout>),
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
            Self::ThreeColumns(cols) => LayoutOutput::all(cols.arrange(area, n)),
            Self::Corner(corner) => LayoutOutput::all(corner.arrange(area, n)),
            Self::Grid => LayoutOutput::all(grid::arrange(area, n)),
            Self::Full => LayoutOutput {
                rects: vec![(focus, area)],
                tabs: None,
            },
            Self::Tabbed { tab_height } => tabbed(area, n, focus, *tab_height),
            Self::Mirror(inner) => inner
                .arrange(area.transposed(), n, focus)
                .map(|r| r.transposed()),
            Self::Reflect(inner) => inner
                .arrange(area, n, focus)
                .map(|r| Rect::new(2 * area.x + area.w - r.x - r.w, r.y, r.w, r.h)),
            Self::NoBorders(inner) => inner.arrange(area, n, focus),
        }
    }

    /// What the layout does with [`PREVIEW_WINDOWS`] windows in
    /// [`PREVIEW_AREA`], window 0 (the master) focused. Drawn by the layout
    /// picker and the pictures in `docs/layouts/`.
    #[must_use]
    pub fn preview(&self) -> LayoutOutput {
        self.arrange(PREVIEW_AREA, PREVIEW_WINDOWS, 0)
    }

    /// Handles a message; returns whether the layout accepted it.
    pub fn handle(&mut self, msg: LayoutMessage, ctx: LayoutContext) -> bool {
        match self {
            Self::Tall(tall) => tall.handle(msg, ctx),
            Self::ThreeColumns(cols) => cols.handle(msg),
            Self::Corner(corner) => ctx.focus.is_some() && corner.handle(msg),
            Self::Grid | Self::Full | Self::Tabbed { .. } => false,
            Self::Mirror(inner) | Self::Reflect(inner) | Self::NoBorders(inner) => {
                inner.handle(msg, ctx)
            }
        }
    }

    /// The layout's name, shown in the bar: plain words for where the
    /// master window goes. Unique within the standard ring, so it also
    /// identifies a layout when restoring a saved ring.
    #[must_use]
    pub fn name(&self) -> String {
        match self {
            Self::Tall(_) => "Tall".to_owned(),
            Self::ThreeColumns(_) => "Three Columns".to_owned(),
            Self::Corner(_) => "Corner".to_owned(),
            Self::Grid => "Grid".to_owned(),
            Self::Full => "Full".to_owned(),
            Self::Tabbed { .. } => "Tabs".to_owned(),
            Self::Mirror(inner) => match **inner {
                Self::Tall(_) => "Wide".to_owned(),
                Self::Grid => "Grid Rows".to_owned(),
                Self::ThreeColumns(_)
                | Self::Corner(_)
                | Self::Full
                | Self::Tabbed { .. }
                | Self::Mirror(_)
                | Self::Reflect(_)
                | Self::NoBorders(_) => format!("{} Mirrored", inner.name()),
            },
            Self::Reflect(inner) => format!("{} Right", inner.name()),
            Self::NoBorders(inner) => inner.name(),
        }
    }

    /// Whether windows arranged by this layout get borders.
    #[must_use]
    pub fn has_borders(&self) -> bool {
        match self {
            Self::NoBorders(_) => false,
            Self::Mirror(inner) | Self::Reflect(inner) => inner.has_borders(),
            Self::Tall(_)
            | Self::ThreeColumns(_)
            | Self::Corner(_)
            | Self::Grid
            | Self::Full
            | Self::Tabbed { .. } => true,
        }
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

/// The reference config's `myLayout` followed by `Corner`, in ring order:
/// `reflectHoriz tiled ||| tiled ||| Mirror tiled ||| noBorders Full |||
/// tabbed ||| Mirror Grid ||| Grid ||| ThreeCol ||| reflectHoriz ThreeCol
/// ||| Corner ||| reflectHoriz Corner` with
/// `tiled = ResizableTall 1 (3/100) (3/5) []` and
/// `ThreeCol 1 (3/100) (1/2)`. Their [names](Layout::name) are unique.
#[must_use]
pub fn standard_layouts(tab_height: i32) -> Vec<Layout> {
    let tiled = || Layout::Tall(Tall::new(1, Ratio::new(3, 100), Ratio::new(3, 5)));
    let three = || Layout::ThreeColumns(ThreeColumns::new(1, Ratio::new(3, 100), Ratio::new(1, 2)));
    let corner = || {
        Layout::Corner(Corner::new(
            Ratio::new(3, 100),
            Ratio::new(2, 3),
            Ratio::new(2, 3),
        ))
    };
    vec![
        Layout::Reflect(Box::new(tiled())),
        tiled(),
        Layout::Mirror(Box::new(tiled())),
        Layout::NoBorders(Box::new(Layout::Full)),
        Layout::Tabbed { tab_height },
        Layout::Mirror(Box::new(Layout::Grid)),
        Layout::Grid,
        three(),
        Layout::Reflect(Box::new(three())),
        corner(),
        Layout::Reflect(Box::new(corner())),
    ]
}

/// Names of the [standard layouts](standard_layouts), in standard order.
#[must_use]
pub fn standard_names() -> Vec<String> {
    standard_layouts(0).iter().map(Layout::name).collect()
}

/// Why a list of layout names is not a valid [`LayoutOrder`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LayoutOrderError {
    /// The list is empty.
    #[error("the list must name at least one layout; valid layouts: {valid}", valid = valid_names())]
    Empty,
    /// A name matches no standard layout.
    #[error("unknown layout `{0}`; valid layouts: {valid}", valid = valid_names())]
    Unknown(String),
    /// A layout is listed twice.
    #[error("layout `{0}` is listed more than once")]
    Duplicate(String),
}

/// The standard layout names, quoted and comma-separated, for messages.
fn valid_names() -> String {
    standard_names()
        .iter()
        .map(|name| format!("\"{name}\""))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Which [standard layouts](standard_layouts) a ring holds and in which
/// order (the config's `layouts`): a non-empty list without duplicates.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LayoutOrder {
    /// Indices into [`standard_layouts`], in ring order.
    indices: Vec<usize>,
}

impl LayoutOrder {
    /// Every standard layout in standard order.
    #[must_use]
    pub fn standard() -> Self {
        Self {
            indices: (0..standard_layouts(0).len()).collect(),
        }
    }

    /// Resolves layout `names` (as [`Layout::name`] shows them, ignoring
    /// case and surrounding whitespace) in ring order.
    pub fn parse<S: AsRef<str>>(names: &[S]) -> Result<Self, LayoutOrderError> {
        if names.is_empty() {
            return Err(LayoutOrderError::Empty);
        }
        let standard = standard_names();
        let mut indices = Vec::with_capacity(names.len());
        for name in names {
            let name = name.as_ref();
            let wanted = name.trim().to_lowercase();
            let index = standard
                .iter()
                .position(|candidate| candidate.to_lowercase() == wanted)
                .ok_or_else(|| LayoutOrderError::Unknown(name.to_owned()))?;
            if indices.contains(&index) {
                return Err(LayoutOrderError::Duplicate(name.to_owned()));
            }
            indices.push(index);
        }
        Ok(Self { indices })
    }

    /// The layouts' names in ring order.
    #[must_use]
    pub fn names(&self) -> Vec<String> {
        let standard = standard_names();
        self.indices
            .iter()
            .filter_map(|&i| standard.get(i).cloned())
            .collect()
    }
}

impl Default for LayoutOrder {
    fn default() -> Self {
        Self::standard()
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

    /// The standard ring: [`standard_layouts`] in order.
    #[must_use]
    pub fn standard(tab_height: i32) -> Self {
        Self::new(standard_layouts(tab_height))
    }

    /// The [standard layouts](standard_layouts) picked and ordered by
    /// `order` (the config's `layouts`), each with its standard parameters;
    /// the first one is active.
    #[must_use]
    pub fn from_order(order: &LayoutOrder, tab_height: i32) -> Self {
        let standard = standard_layouts(tab_height);
        Self::new(
            order
                .indices
                .iter()
                .filter_map(|&i| standard.get(i).cloned())
                .collect(),
        )
    }

    /// The ring to use for a workspace whose ring was `saved` (e.g. before
    /// a restart), with `self` being the configured ring.
    ///
    /// If `saved` has the same layouts as `self` (by name), it is
    /// kept with every entry's state; only the `tabbed` entries take the
    /// configured tab height. Otherwise `self` is used, switched to the
    /// layout `saved` had active if the configured ring has one with that
    /// name.
    #[must_use]
    pub fn restore(&self, saved: &Self) -> Self {
        let compatible = saved.current < saved.layouts.len()
            && saved.layouts.len() == self.layouts.len()
            && saved
                .layouts
                .iter()
                .zip(&self.layouts)
                .all(|(a, b)| a.name() == b.name());
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
        if let Some(active) = saved.layouts.get(saved.current).map(Layout::name)
            && let Some(index) = ring.layouts.iter().position(|l| l.name() == active)
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

    /// Every layout of the ring, in ring order.
    #[must_use]
    pub fn layouts(&self) -> &[Layout] {
        &self.layouts
    }

    /// Makes the layout at `index` active, keeping every entry's state;
    /// returns `false` (and changes nothing) if there is no such entry.
    pub fn select(&mut self, index: usize) -> bool {
        let valid = index < self.layouts.len();
        if valid {
            self.current = index;
        }
        valid
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
