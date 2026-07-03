//! Window tiling, focus management and border rendering.
//!
//! Each [`Workspace`] carries a [`LayoutCycle`]: an ordered ring of nine layout
//! instances reproducing the user's xmonad `myLayout` choice combinator
//! (`reflectHoriz tiled ||| tiled ||| Mirror tiled ||| noBorders Full |||
//! tabbed ||| Mirror Grid ||| Grid ||| ThreeCol ||| reflectHoriz ThreeCol`).
//! Each layout keeps its own state; `NextLayout` advances the ring without
//! resetting it and `ResetLayout` restores the whole ring to defaults. The base
//! tilers are ported directly from the xmonad and xmonad-contrib Haskell
//! sources (see `resources/`).

use smithay::{
    backend::renderer::{
        Color32F,
        element::{
            Kind,
            solid::{
                SolidColorBuffer,
                SolidColorRenderElement,
            },
        },
    },
    desktop::{
        Window,
        layer_map_for_output,
    },
    output::Output,
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{
        IsAlive,
        Logical,
        Point,
        Rectangle,
        SERIAL_COUNTER,
        Size,
    },
};

use crate::{
    Oxmonad,
    config,
};

/// A window together with its computed location and size.
type Placement = (Window, Point<i32, Logical>, Size<i32, Logical>);

// Tabbed decoration colors, from the default xmonad `Theme` (the user's
// `myTabConf` only overrides the font). See
// `resources/xmonad-contrib/XMonad/Layout/Decoration.hs`: activeColor
// "#999999", inactiveColor "#666666", activeTextColor "#FFFFFF",
// inactiveTextColor "#BFBFBF". Defined here rather than `config.rs` to keep the
// in-flight config change conflict-free.

/// Background color of the focused (active) tab — `#999999`.
pub(crate) const TAB_ACTIVE_BG: Color32F = Color32F::new(0.6, 0.6, 0.6, 1.0);
/// Text color of the focused (active) tab — `#FFFFFF`.
pub(crate) const TAB_ACTIVE_TEXT: Color32F = Color32F::new(1.0, 1.0, 1.0, 1.0);
/// Background color of an unfocused tab — `#666666`.
pub(crate) const TAB_INACTIVE_BG: Color32F = Color32F::new(0.4, 0.4, 0.4, 1.0);
/// Text color of an unfocused tab — `#BFBFBF`.
pub(crate) const TAB_INACTIVE_TEXT: Color32F =
    Color32F::new(0.749_019_6, 0.749_019_6, 0.749_019_6, 1.0);
/// Height of the tab strip, in logical pixels — the default `Theme`
/// `decoHeight` of 20, which comfortably fits the 14px-tall (7px glyphs at 2x)
/// bitmap font.
pub(crate) const TAB_BAR_HEIGHT: i32 = 20;

/// A virtual desktop: an independent window stack and layout ring, like an
/// xmonad workspace.
///
/// Windows belong to exactly one workspace. A workspace is shown when a
/// [`Monitor`] displays it; otherwise it is hidden and its windows are
/// unmapped. Workspaces are addressed by index (their position in
/// [`Oxmonad::workspaces`]) and carry a [name](Self::name) for display —
/// `"1"`..`"9"` by default, or any custom label from [`config::WORKSPACES`].
#[derive(Debug)]
pub struct Workspace {
    /// Display name, e.g. `"1"` or `"web"`.
    pub name: String,
    /// Windows on this workspace in tiling order; index 0 is the master.
    pub windows: Vec<Window>,
    /// The layout ring used to arrange this workspace's windows.
    pub layout: LayoutCycle,
}

impl Workspace {
    /// Creates an empty workspace with the given display `name`.
    #[must_use]
    pub fn new(name: String) -> Self {
        Self {
            name,
            windows: Vec::new(),
            layout: LayoutCycle::default(),
        }
    }
}

/// One monitor: an output and the index of the [`Workspace`] it currently
/// displays.
///
/// Each monitor shows exactly one workspace, and no two monitors show the same
/// one. Switching the displayed workspace is done with
/// [`Oxmonad::view_workspace`]; windows are sent to a workspace with
/// [`Oxmonad::move_focused_to_workspace`].
#[derive(Debug)]
pub struct Monitor {
    /// The output this monitor renders to.
    pub output: Output,
    /// Index into [`Oxmonad::workspaces`] of the workspace shown here.
    pub workspace: usize,
}

/// The ordered ring of layouts a workspace cycles through, mirroring xmonad's
/// `l ||| r ||| …` choice combinator: each entry keeps its own state and the
/// [current](Self::current) one arranges the windows.
#[derive(Debug)]
pub struct LayoutCycle {
    /// The nine layout instances, in `myLayout` order.
    layouts: Vec<Layout>,
    /// Index into [`Self::layouts`] of the active layout.
    current: usize,
}

impl Default for LayoutCycle {
    fn default() -> Self {
        Self {
            layouts: default_layouts(),
            current: 0,
        }
    }
}

/// The default nine-layout ring, matching the user's `myLayout`.
fn default_layouts() -> Vec<Layout> {
    vec![
        Layout::ReflectTall(ResizableTall::default()),
        Layout::Tall(ResizableTall::default()),
        Layout::MirrorTall(ResizableTall::default()),
        Layout::Full,
        Layout::Tabbed,
        Layout::MirrorGrid(Grid::default()),
        Layout::Grid(Grid::default()),
        Layout::ThreeCol(ThreeCol::default()),
        Layout::ReflectThreeCol(ThreeCol::default()),
    ]
}

impl LayoutCycle {
    /// The active layout.
    fn current(&self) -> &Layout {
        &self.layouts[self.current.min(self.layouts.len().saturating_sub(1))]
    }

    /// The active layout, mutably.
    fn current_mut(&mut self) -> &mut Layout {
        let index = self.current.min(self.layouts.len().saturating_sub(1));
        &mut self.layouts[index]
    }

    /// Advances to the next layout in the ring, wrapping, without resetting any
    /// layout's state (xmonad's `NextLayout`).
    fn next(&mut self) {
        if !self.layouts.is_empty() {
            self.current = (self.current + 1) % self.layouts.len();
        }
    }

    /// Restores the whole ring to defaults: first layout, fresh states.
    fn reset(&mut self) {
        self.layouts = default_layouts();
        self.current = 0;
    }

    /// Short display name of the active layout, for the status bar.
    #[must_use]
    pub(crate) fn name(&self) -> &'static str {
        self.current().name()
    }

    /// Whether the active layout draws no window borders (`noBorders Full`).
    #[must_use]
    pub(crate) fn is_full(&self) -> bool {
        matches!(self.current(), Layout::Full)
    }

    /// Whether the active layout is the tabbed layout.
    #[must_use]
    pub(crate) fn is_tabbed(&self) -> bool {
        matches!(self.current(), Layout::Tabbed)
    }

    /// Whether the active layout shows only the focused window
    /// (`Full`/`Tabbed`), so a focus change must retile to reveal the new
    /// window.
    #[must_use]
    pub(crate) fn only_focused_visible(&self) -> bool {
        matches!(self.current(), Layout::Full | Layout::Tabbed)
    }

    fn arrange(
        &self,
        n: usize,
        focused: usize,
        area: Rectangle<i32, Logical>,
    ) -> Vec<(usize, Rectangle<i32, Logical>)> {
        self.current().arrange(n, focused, area)
    }

    fn expand_master(&mut self) {
        self.current_mut().expand_master();
    }

    fn shrink_master(&mut self) {
        self.current_mut().shrink_master();
    }

    fn expand_focused(&mut self, index: usize, count: usize) {
        self.current_mut().expand_focused(index, count);
    }

    fn shrink_focused(&mut self, index: usize, count: usize) {
        self.current_mut().shrink_focused(index, count);
    }

    fn inc_master(&mut self, delta: i32) {
        self.current_mut().inc_master(delta);
    }
}

/// One layout in the ring. Reflect/Mirror variants wrap a base tiler and
/// transform its rectangles; `Full`/`Tabbed` place only the focused window.
#[derive(Debug)]
enum Layout {
    /// `reflectHoriz tiled`: master column on the right, stack on the left.
    ReflectTall(ResizableTall),
    /// The plain `ResizableTall`: master column on the left.
    Tall(ResizableTall),
    /// `Mirror tiled`: master row on top, stack below.
    MirrorTall(ResizableTall),
    /// `noBorders Full`: only the focused window, fullscreen, no borders.
    Full,
    /// `tabbed`: only the focused window's body, with a tab strip on top.
    Tabbed,
    /// `Mirror Grid`: the grid transposed.
    MirrorGrid(Grid),
    /// xmonad-contrib `Grid`.
    Grid(Grid),
    /// `ThreeCol`: master column on the left, two stack columns to the right.
    ThreeCol(ThreeCol),
    /// `reflectHoriz ThreeCol`: the three-column layout mirrored horizontally.
    ReflectThreeCol(ThreeCol),
}

impl Layout {
    fn name(&self) -> &'static str {
        match self {
            Layout::ReflectTall(_) => "ReflectTall",
            Layout::Tall(_) => "Tall",
            Layout::MirrorTall(_) => "Mirror Tall",
            Layout::Full => "Full",
            Layout::Tabbed => "Tabbed",
            Layout::MirrorGrid(_) => "Mirror Grid",
            Layout::Grid(_) => "Grid",
            Layout::ThreeCol(_) => "ThreeCol",
            Layout::ReflectThreeCol(_) => "ReflectThreeCol",
        }
    }

    /// Places the `n` windows of a workspace, returning `(window-index, tile)`
    /// pairs. Tilers return all `n` indices in order; `Full`/`Tabbed` return
    /// only the `focused` window (others are left unmapped).
    fn arrange(
        &self,
        n: usize,
        focused: usize,
        area: Rectangle<i32, Logical>,
    ) -> Vec<(usize, Rectangle<i32, Logical>)> {
        match self {
            Layout::Tall(tall) => index_all(tall.arrange(n, area)),
            Layout::ReflectTall(tall) => {
                index_all(reflect_horizontally(area, tall.arrange(n, area)))
            }
            Layout::MirrorTall(tall) => index_all(mirrored(area, |a| tall.arrange(n, a))),
            Layout::Grid(grid) => index_all(grid.arrange(n, area)),
            Layout::MirrorGrid(grid) => index_all(mirrored(area, |a| grid.arrange(n, a))),
            Layout::ThreeCol(cols) => index_all(cols.arrange(n, area)),
            Layout::ReflectThreeCol(cols) => {
                index_all(reflect_horizontally(area, cols.arrange(n, area)))
            }
            Layout::Full => full_arrange(n, focused, area),
            Layout::Tabbed => tabbed_arrange(n, focused, area),
        }
    }

    fn expand_master(&mut self) {
        match self {
            Layout::ReflectTall(tall) | Layout::Tall(tall) | Layout::MirrorTall(tall) => {
                tall.expand_master();
            }
            Layout::ThreeCol(cols) | Layout::ReflectThreeCol(cols) => cols.expand(),
            Layout::Full | Layout::Tabbed | Layout::Grid(_) | Layout::MirrorGrid(_) => {}
        }
    }

    fn shrink_master(&mut self) {
        match self {
            Layout::ReflectTall(tall) | Layout::Tall(tall) | Layout::MirrorTall(tall) => {
                tall.shrink_master();
            }
            Layout::ThreeCol(cols) | Layout::ReflectThreeCol(cols) => cols.shrink(),
            Layout::Full | Layout::Tabbed | Layout::Grid(_) | Layout::MirrorGrid(_) => {}
        }
    }

    fn expand_focused(&mut self, index: usize, count: usize) {
        match self {
            Layout::ReflectTall(tall) | Layout::Tall(tall) | Layout::MirrorTall(tall) => {
                tall.expand_window(index, count);
            }
            Layout::Full
            | Layout::Tabbed
            | Layout::Grid(_)
            | Layout::MirrorGrid(_)
            | Layout::ThreeCol(_)
            | Layout::ReflectThreeCol(_) => {}
        }
    }

    fn shrink_focused(&mut self, index: usize, count: usize) {
        match self {
            Layout::ReflectTall(tall) | Layout::Tall(tall) | Layout::MirrorTall(tall) => {
                tall.shrink_window(index, count);
            }
            Layout::Full
            | Layout::Tabbed
            | Layout::Grid(_)
            | Layout::MirrorGrid(_)
            | Layout::ThreeCol(_)
            | Layout::ReflectThreeCol(_) => {}
        }
    }

    fn inc_master(&mut self, delta: i32) {
        match self {
            Layout::ReflectTall(tall) | Layout::Tall(tall) | Layout::MirrorTall(tall) => {
                tall.inc_master(delta);
            }
            Layout::ThreeCol(cols) | Layout::ReflectThreeCol(cols) => cols.inc_master(delta),
            Layout::Full | Layout::Tabbed | Layout::Grid(_) | Layout::MirrorGrid(_) => {}
        }
    }
}

/// Pairs each rectangle with its window index, in order (for full tilers).
fn index_all(rects: Vec<Rectangle<i32, Logical>>) -> Vec<(usize, Rectangle<i32, Logical>)> {
    rects.into_iter().enumerate().collect()
}

/// Places only the focused window fullscreen (`Full`).
fn full_arrange(
    n: usize,
    focused: usize,
    area: Rectangle<i32, Logical>,
) -> Vec<(usize, Rectangle<i32, Logical>)> {
    if n == 0 {
        return Vec::new();
    }
    vec![(focused.min(n - 1), area)]
}

/// Places only the focused window below the tab strip (`Tabbed`).
fn tabbed_arrange(
    n: usize,
    focused: usize,
    area: Rectangle<i32, Logical>,
) -> Vec<(usize, Rectangle<i32, Logical>)> {
    if n == 0 {
        return Vec::new();
    }
    let index = focused.min(n - 1);
    // With a single window the tab bar is hidden (xmonad's `WhenPlural`), so the
    // window fills the whole area.
    if n < 2 {
        return vec![(index, area)];
    }
    let body = Rectangle::new(
        Point::from((area.loc.x, area.loc.y + TAB_BAR_HEIGHT)),
        Size::from((area.size.w, (area.size.h - TAB_BAR_HEIGHT).max(1))),
    );
    vec![(index, body)]
}

/// Runs `layout` on the transposed `area` and transposes each result back,
/// reproducing xmonad's `Mirror` (`mirrorRect` swaps x↔y and w↔h).
fn mirrored<F>(area: Rectangle<i32, Logical>, layout: F) -> Vec<Rectangle<i32, Logical>>
where
    F: FnOnce(Rectangle<i32, Logical>) -> Vec<Rectangle<i32, Logical>>,
{
    layout(transpose(area)).into_iter().map(transpose).collect()
}

/// Swaps a rectangle's axes: xmonad's `mirrorRect (Rectangle rx ry rw rh) =
/// Rectangle ry rx rh rw`.
fn transpose(rect: Rectangle<i32, Logical>) -> Rectangle<i32, Logical> {
    Rectangle::new(
        Point::from((rect.loc.y, rect.loc.x)),
        Size::from((rect.size.h, rect.size.w)),
    )
}

/// Reflects each rectangle horizontally within `area`: xmonad's `reflectRect
/// Horiz`, `x' = 2*ax + aw - rx - rw`.
fn reflect_horizontally(
    area: Rectangle<i32, Logical>,
    rects: Vec<Rectangle<i32, Logical>>,
) -> Vec<Rectangle<i32, Logical>> {
    rects
        .into_iter()
        .map(|rect| {
            let x = 2 * area.loc.x + area.size.w - rect.loc.x - rect.size.w;
            Rectangle::new(Point::from((x, rect.loc.y)), rect.size)
        })
        .collect()
}

/// A master/stack tiling layout (xmonad's `ResizableTall`).
#[derive(Debug, Clone)]
pub struct ResizableTall {
    /// Number of windows in the master column.
    pub nmaster: usize,
    /// Fraction of the screen width occupied by the master column.
    pub ratio: f64,
    /// Amount to change the focused window's row weight by when resizing.
    pub delta: f64,
    /// Per-window vertical row weights in tiling order.
    row_weights: Vec<f64>,
}

impl Default for ResizableTall {
    fn default() -> Self {
        // Matches the user's xmonad config: ResizableTall 1 (3/100) (3/5).
        Self {
            nmaster: 1,
            ratio: 3.0 / 5.0,
            delta: 3.0 / 100.0,
            row_weights: Vec::new(),
        }
    }
}

impl ResizableTall {
    /// Computes a tile rectangle for each of `n` windows within `area`.
    ///
    /// The returned vector is in window order: index 0 is the master-most
    /// window. Returns an empty vector when `n == 0`.
    #[must_use]
    pub fn arrange(&self, n: usize, area: Rectangle<i32, Logical>) -> Vec<Rectangle<i32, Logical>> {
        let mut tiles = Vec::with_capacity(n);
        if n == 0 {
            return tiles;
        }

        let nmaster = self.nmaster.min(n);
        let n_stack = n - nmaster;
        let weights = self.weights(n);

        // With `nmaster == 0` every window shares the single stack column.
        if nmaster == 0 {
            split_column(area.loc, area.size, &weights, &mut tiles);
            return tiles;
        }

        // The master column takes the full width when there is no stack.
        let master_width = if n_stack == 0 {
            area.size.w
        } else {
            scale_to_i32(f64::from(area.size.w) * self.ratio)
        };

        split_column(
            Point::from((area.loc.x, area.loc.y)),
            Size::from((master_width, area.size.h)),
            &weights[0..nmaster],
            &mut tiles,
        );

        if n_stack > 0 {
            split_column(
                Point::from((area.loc.x + master_width, area.loc.y)),
                Size::from((area.size.w - master_width, area.size.h)),
                &weights[nmaster..n],
                &mut tiles,
            );
        }

        tiles
    }

    /// Increases the vertical share of the window at `index`.
    pub fn expand_window(&mut self, index: usize, window_count: usize) {
        self.resize_window(index, window_count, self.delta);
    }

    /// Decreases the vertical share of the window at `index`.
    pub fn shrink_window(&mut self, index: usize, window_count: usize) {
        self.resize_window(index, window_count, -self.delta);
    }

    /// Makes the master area wider by one resize step.
    pub fn expand_master(&mut self) {
        self.ratio = (self.ratio + self.delta).min(0.9);
    }

    /// Makes the master area narrower by one resize step.
    pub fn shrink_master(&mut self) {
        self.ratio = (self.ratio - self.delta).max(0.1);
    }

    /// Changes the master count by `delta`, clamped at zero (xmonad's
    /// `IncMasterN`: `max 0 (nmaster + d)`).
    pub fn inc_master(&mut self, delta: i32) {
        self.nmaster = adjust_count(self.nmaster, delta);
    }

    fn resize_window(&mut self, index: usize, window_count: usize, delta: f64) {
        if index >= window_count {
            return;
        }

        self.ensure_weights(window_count);
        self.row_weights[index] = (self.row_weights[index] + delta).max(0.1);
    }

    fn weights(&self, n: usize) -> Vec<f64> {
        let mut weights = self.row_weights.clone();
        weights.resize(n, 1.0);
        weights
    }

    fn ensure_weights(&mut self, n: usize) {
        self.row_weights.resize(n, 1.0);
        self.row_weights.truncate(n);
    }
}

/// A square-ish grid layout (xmonad-contrib's `Grid`).
#[derive(Debug, Clone)]
struct Grid {
    /// Aspect ratio the grid strives for (`GridRatio`).
    ratio: f64,
}

impl Default for Grid {
    fn default() -> Self {
        // xmonad-contrib's `defaultRatio = 16/9` (bare `Grid`).
        Self { ratio: 16.0 / 9.0 }
    }
}

impl Grid {
    /// Ports `XMonad.Layout.Grid.arrange`: pick a column count from the aspect
    /// ratio, fill columns top-to-bottom, and give the rightmost columns one
    /// extra window each.
    fn arrange(&self, n: usize, area: Rectangle<i32, Logical>) -> Vec<Rectangle<i32, Logical>> {
        if n == 0 {
            return Vec::new();
        }
        let nwins = count_i32(n);
        let width = f64::from(area.size.w);
        let height = f64::from(area.size.h);
        // ncols = max 1 . min nwins . round . sqrt $ nwins * rw / (rh * ratio)
        let ncols = round_to_i32((count_f64(n) * width / (height * self.ratio)).sqrt())
            .clamp(1, nwins.max(1));
        let mincs = (nwins / ncols).max(1);
        let extrs = nwins - ncols * mincs;

        let xcoords = chop(ncols, area.size.w);
        let ycoords = chop(mincs, area.size.h);
        let ycoords_extra = chop(mincs + 1, area.size.h);

        let split = usize::try_from((ncols - extrs).max(0))
            .unwrap_or(0)
            .min(xcoords.len());
        let (xbase, xext) = xcoords.split_at(split);

        let mut out = Vec::with_capacity(n);
        combine(&mut out, area, &ycoords, xbase);
        combine(&mut out, area, &ycoords_extra, xext);
        out
    }
}

/// Column-major fill: for each column `(x, w)`, one rectangle per row `(y, h)`.
fn combine(
    out: &mut Vec<Rectangle<i32, Logical>>,
    area: Rectangle<i32, Logical>,
    ys: &[(i32, i32)],
    xs: &[(i32, i32)],
) {
    for &(x, w) in xs {
        for &(y, h) in ys {
            out.push(Rectangle::new(
                Point::from((area.loc.x + x, area.loc.y + y)),
                Size::from((w, h)),
            ));
        }
    }
}

/// Splits `m` into `n` `(position, size)` cells, giving the first cell the
/// rounding remainder (xmonad-contrib's `Grid.chop`).
fn chop(n: i32, m: i32) -> Vec<(i32, i32)> {
    let n = n.max(1);
    let k = m / n;
    let mut out = Vec::with_capacity(usize::try_from(n).unwrap_or(0));
    out.push((0, m - k * (n - 1)));
    let mut i = 1;
    while i < n {
        out.push((m - (n - i) * k, k));
        i += 1;
    }
    out
}

/// A three-column layout (xmonad-contrib's plain `ThreeCol`, master on the
/// left).
#[derive(Debug, Clone)]
struct ThreeCol {
    /// Number of windows in the master column.
    nmaster: usize,
    /// Amount to change the master fraction by when resizing.
    delta: f64,
    /// Fraction of the width devoted to the master column.
    frac: f64,
}

impl Default for ThreeCol {
    fn default() -> Self {
        // Matches the user's xmonad config: ThreeCol 1 (3/100) (1/2).
        Self {
            nmaster: 1,
            delta: 3.0 / 100.0,
            frac: 1.0 / 2.0,
        }
    }
}

impl ThreeCol {
    /// Ports `XMonad.Layout.ThreeColumns.tile3` with `middle = False`.
    fn arrange(&self, n: usize, area: Rectangle<i32, Logical>) -> Vec<Rectangle<i32, Logical>> {
        if n == 0 {
            return Vec::new();
        }
        let nmaster = self.nmaster;
        let frac = self.frac;

        if n <= nmaster || nmaster == 0 {
            return split_vertically(n, area);
        }

        if n <= nmaster + 1 {
            let f = if frac < 0.0 { 1.0 + frac } else { frac };
            let (s1, s2) = split_horizontally_by(f, area);
            let mut out = split_vertically(nmaster, s1);
            out.extend(split_vertically(n - nmaster, s2));
            return out;
        }

        let f = if frac < 0.0 { 1.0 + 2.0 * frac } else { frac };
        let (r1, r2, r3) = split3_horizontally_by(f, area);
        let nslave = n - nmaster;
        let nslave1 = nslave.div_ceil(2);
        let nslave2 = n - nmaster - nslave1;

        let mut out = split_vertically(nmaster, r1);
        out.extend(split_vertically(nslave1, r2));
        out.extend(split_vertically(nslave2, r3));
        out
    }

    /// `Expand`: `frac = min 1 (frac + delta)`.
    fn expand(&mut self) {
        self.frac = (self.frac + self.delta).min(1.0);
    }

    /// `Shrink`: `frac = max (-0.5) (frac - delta)`.
    fn shrink(&mut self) {
        self.frac = (self.frac - self.delta).max(-0.5);
    }

    /// `IncMasterN`: `nmaster = max 0 (nmaster + d)`.
    fn inc_master(&mut self, delta: i32) {
        self.nmaster = adjust_count(self.nmaster, delta);
    }
}

/// Divides `area` into `count` stacked rows, mirroring xmonad's recursive
/// `splitVertically` (each row is the remaining height divided by the remaining
/// count, so the last row absorbs the remainder).
fn split_vertically(count: usize, area: Rectangle<i32, Logical>) -> Vec<Rectangle<i32, Logical>> {
    let mut out = Vec::with_capacity(count);
    if count == 0 {
        return out;
    }
    let mut y = area.loc.y;
    let mut height = area.size.h;
    let mut remaining = count;
    while remaining > 1 {
        let divisor = count_i32(remaining).max(1);
        let small = height / divisor;
        out.push(Rectangle::new(
            Point::from((area.loc.x, y)),
            Size::from((area.size.w, small)),
        ));
        y += small;
        height -= small;
        remaining -= 1;
    }
    out.push(Rectangle::new(
        Point::from((area.loc.x, y)),
        Size::from((area.size.w, height)),
    ));
    out
}

/// Splits `area` left/right at fraction `f`: `leftw = floor(sw * f)` (xmonad's
/// `splitHorizontallyBy`).
fn split_horizontally_by(
    f: f64,
    area: Rectangle<i32, Logical>,
) -> (Rectangle<i32, Logical>, Rectangle<i32, Logical>) {
    let leftw = floor_to_i32(f64::from(area.size.w) * f);
    (
        Rectangle::new(area.loc, Size::from((leftw, area.size.h))),
        Rectangle::new(
            Point::from((area.loc.x + leftw, area.loc.y)),
            Size::from((area.size.w - leftw, area.size.h)),
        ),
    )
}

/// Splits `area` into three columns for `ThreeCol` (master left): `r1w =
/// ceil(sw*f)`, then the remainder split into two equal columns.
fn split3_horizontally_by(
    f: f64,
    area: Rectangle<i32, Logical>,
) -> (
    Rectangle<i32, Logical>,
    Rectangle<i32, Logical>,
    Rectangle<i32, Logical>,
) {
    let sw = area.size.w;
    let r1w = ceil_to_i32(f64::from(sw) * f);
    let r2w = (sw - r1w + 1) / 2;
    let r3w = sw - r1w - r2w;
    let height = area.size.h;
    (
        Rectangle::new(area.loc, Size::from((r1w, height))),
        Rectangle::new(
            Point::from((area.loc.x + r1w, area.loc.y)),
            Size::from((r2w, height)),
        ),
        Rectangle::new(
            Point::from((area.loc.x + r1w + r2w, area.loc.y)),
            Size::from((r3w, height)),
        ),
    )
}

/// Splits a column into `count` equal-height rows, pushing them onto `out`.
///
/// The last row absorbs any rounding remainder so the rows exactly fill the
/// column height. Used by [`ResizableTall`] with its per-row weights.
fn split_column(
    loc: Point<i32, Logical>,
    size: Size<i32, Logical>,
    weights: &[f64],
    out: &mut Vec<Rectangle<i32, Logical>>,
) {
    debug_assert!(!weights.is_empty());
    let total_weight: f64 = weights.iter().sum();
    let mut y = loc.y;
    let mut used_height = 0;

    for (i, weight) in weights.iter().enumerate() {
        let height = if i == weights.len() - 1 {
            size.h - used_height
        } else {
            scale_to_i32(f64::from(size.h) * weight / total_weight)
        };
        out.push(Rectangle::new(
            Point::from((loc.x, y)),
            Size::from((size.w, height)),
        ));
        y += height;
        used_height += height;
    }
}

/// Adjusts a master count by `delta`, clamped at zero (xmonad's `max 0 (n+d)`).
fn adjust_count(count: usize, delta: i32) -> usize {
    let current = i32::try_from(count).unwrap_or(i32::MAX);
    usize::try_from((current + delta).max(0)).unwrap_or(0)
}

/// A window count as `f64`, without a lint-triggering `as` cast.
fn count_f64(n: usize) -> f64 {
    f64::from(u32::try_from(n).unwrap_or(u32::MAX))
}

/// A window count as `i32`, saturating (counts are tiny in practice).
fn count_i32(n: usize) -> i32 {
    i32::try_from(n).unwrap_or(i32::MAX)
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "Pixel dimensions are screen-bounded, non-negative integers."
)]
fn scale_to_i32(value: f64) -> i32 {
    value.round() as i32
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "A grid column count is a small non-negative integer."
)]
fn round_to_i32(value: f64) -> i32 {
    value.round_ties_even() as i32
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "Pixel dimensions are screen-bounded, non-negative integers."
)]
fn floor_to_i32(value: f64) -> i32 {
    value.floor() as i32
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "Pixel dimensions are screen-bounded, non-negative integers."
)]
fn ceil_to_i32(value: f64) -> i32 {
    value.ceil() as i32
}

impl Oxmonad {
    /// The windows of `workspace` to tile, in order: its window stack minus
    /// the floating layer.
    pub(crate) fn tiling_order(&self, workspace: &Workspace) -> Vec<Window> {
        workspace
            .windows
            .iter()
            .filter(|window| !self.floating.contains_key(*window))
            .cloned()
            .collect()
    }

    /// Registers a freshly-detected `output` as a new monitor, assigning it the
    /// first workspace not already shown elsewhere.
    pub fn add_monitor(&mut self, output: Output) {
        let workspace = (0..self.workspaces.len())
            .find(|index| !self.monitors.iter().any(|m| m.workspace == *index))
            .unwrap_or(0);
        self.monitors.push(Monitor { output, workspace });
    }

    /// Iterates every managed window across all workspaces (visible or hidden).
    pub fn managed_windows(&self) -> impl Iterator<Item = &Window> {
        self.workspaces
            .iter()
            .flat_map(|workspace| workspace.windows.iter())
    }

    /// Whether `output` is the primary monitor — the one carrying the bar.
    ///
    /// The primary is [`config::PRIMARY_OUTPUT`] when a connected output
    /// matches that name, otherwise the first detected output.
    #[must_use]
    pub fn is_primary_output(&self, output: &Output) -> bool {
        let name = output.name();
        match config::PRIMARY_OUTPUT {
            Some(primary) if self.monitors.iter().any(|m| m.output.name() == primary) => {
                name == primary
            }
            _ => self
                .monitors
                .first()
                .is_some_and(|m| m.output.name() == name),
        }
    }

    /// Index into [`Self::workspaces`] of the workspace holding `window`.
    ///
    /// A copied window lives on several workspaces; this returns the first by
    /// index. Prefer [`Self::visible_workspace_of_window`] or
    /// [`Self::source_workspace_of_window`] where the distinction matters.
    fn workspace_index_of_window(&self, window: &Window) -> Option<usize> {
        self.workspaces
            .iter()
            .position(|workspace| workspace.windows.contains(window))
    }

    /// The workspace holding `window` that is shown on a monitor, together
    /// with that monitor's index — preferring a visible copy over hidden ones.
    pub(crate) fn visible_workspace_of_window(&self, window: &Window) -> Option<(usize, usize)> {
        self.workspaces
            .iter()
            .enumerate()
            .filter(|(_, workspace)| workspace.windows.contains(window))
            .find_map(|(index, _)| self.monitor_showing(index).map(|monitor| (index, monitor)))
    }

    /// The workspace a move of `window` should take it *from*: the current
    /// workspace when it holds the window (the copy the user is looking at),
    /// otherwise the first workspace containing it.
    fn source_workspace_of_window(&self, window: &Window) -> Option<usize> {
        self.current_workspace()
            .filter(|&index| {
                self.workspaces
                    .get(index)
                    .is_some_and(|workspace| workspace.windows.contains(window))
            })
            .or_else(|| self.workspace_index_of_window(window))
    }

    /// Index into [`Self::workspaces`] of the workspace shown on the active
    /// monitor (the "current" workspace).
    #[must_use]
    pub fn current_workspace(&self) -> Option<usize> {
        self.monitors.get(self.active_monitor).map(|m| m.workspace)
    }

    /// The monitor currently displaying workspace `index`, if any.
    fn monitor_showing(&self, index: usize) -> Option<usize> {
        self.monitors.iter().position(|m| m.workspace == index)
    }

    /// The layout area for `monitor` in global coordinates: its output's
    /// non-exclusive zone, minus the bar strip on the primary monitor.
    pub(crate) fn tiling_area(&self, monitor: &Monitor) -> Option<Rectangle<i32, Logical>> {
        let output_geo = self.space.output_geometry(&monitor.output)?;
        // `non_exclusive_zone` is output-local; shift it into global space.
        let mut area = layer_map_for_output(&monitor.output).non_exclusive_zone();
        area.loc += output_geo.loc;
        // Only the primary monitor gives up a strip for the built-in bar.
        if self.bar_visible && self.is_primary_output(&monitor.output) {
            Some(crate::bar::content_area(area))
        } else {
            Some(area)
        }
    }

    /// Recomputes the layout: tiles every visible workspace on its monitor and
    /// unmaps windows the layout does not place (hidden workspaces, and the
    /// non-focused windows of `Full`/`Tabbed`).
    pub fn relayout(&mut self) {
        for workspace in &mut self.workspaces {
            workspace.windows.retain(IsAlive::alive);
        }
        self.floating.retain(|window, _| window.alive());
        if self
            .selected_window
            .as_ref()
            .is_some_and(|window| !window.alive())
        {
            self.selected_window = None;
        }

        let border = config::BORDER_WIDTH;

        // Collect placements first so we don't borrow the workspaces while
        // mutating `self.space`. The active monitor is processed last: a
        // window copied onto two *visible* workspaces can only be shown in one
        // place, and the last-written placement wins, so the copy the user is
        // interacting with is the one that shows (matching xmonad's own
        // single-position limitation for CopyWindow).
        let mut monitor_order: Vec<usize> = (0..self.monitors.len()).collect();
        monitor_order.sort_by_key(|&index| usize::from(index == self.active_monitor));
        let mut placements: Vec<Placement> = Vec::new();
        for monitor_index in monitor_order {
            let monitor = &self.monitors[monitor_index];
            let Some(workspace) = self.workspaces.get(monitor.workspace) else {
                continue;
            };
            let Some(area) = self.tiling_area(monitor) else {
                continue;
            };
            let windows = self.tiling_order(workspace);
            let n = windows.len();
            let focused = self.focus_index_within(&windows);
            // `Full` (noBorders) fills the tile edge-to-edge; everything else
            // insets by the border width to leave room for the border ring.
            let inset = !workspace.layout.is_full();
            for (index, tile) in workspace.layout.arrange(n, focused, area) {
                let Some(window) = windows.get(index) else {
                    continue;
                };
                let geometry = if inset {
                    window_geometry(tile, border)
                } else {
                    tile
                };
                placements.push((window.clone(), geometry.loc, geometry.size));
            }
        }

        // Floating windows on visible workspaces keep their own geometry and
        // are mapped after the tiled ones, putting them on top of the stack.
        let mut float_placements: Vec<(Window, Point<i32, Logical>)> = Vec::new();
        for monitor in &self.monitors {
            let Some(workspace) = self.workspaces.get(monitor.workspace) else {
                continue;
            };
            for window in &workspace.windows {
                if let Some(geometry) = self.floating.get(window) {
                    float_placements.push((window.clone(), geometry.loc));
                }
            }
        }

        // Anything not placed this pass (hidden workspaces, unfocused
        // `Full`/`Tabbed` windows) is unmapped so it stops rendering.
        let placed: Vec<Window> = placements
            .iter()
            .map(|(window, ..)| window.clone())
            .chain(float_placements.iter().map(|(window, _)| window.clone()))
            .collect();
        let to_unmap: Vec<Window> = self
            .managed_windows()
            .filter(|window| !placed.iter().any(|placed| placed == *window))
            .cloned()
            .collect();
        for window in to_unmap {
            self.space.unmap_elem(&window);
        }

        for (window, loc, size) in placements {
            if let Some(toplevel) = window.toplevel() {
                toplevel.with_pending_state(|state| state.size = Some(size));
                toplevel.send_pending_configure();
            }
            self.space.map_element(window, loc, false);
        }

        // Floating windows are not resized by the layout; mapping them last
        // keeps the floating layer above the tiled one.
        for (window, loc) in float_placements {
            self.space.map_element(window, loc, false);
        }
    }

    /// Returns the currently selected managed window, if any.
    #[must_use]
    pub fn focused_window(&self) -> Option<Window> {
        if let Some(window) = self
            .selected_window
            .as_ref()
            .filter(|window| IsAlive::alive(*window))
            .cloned()
        {
            return Some(window);
        }

        let focus = self.seat.get_keyboard()?.current_focus()?;
        self.managed_windows()
            .find(|window| {
                window
                    .toplevel()
                    .is_some_and(|toplevel| toplevel.wl_surface() == &focus)
            })
            .cloned()
    }

    /// Gives keyboard focus to `window`, raising it and updating activation.
    ///
    /// Focus follows the monitor: the monitor showing the window's workspace
    /// becomes active. When the window's workspace uses a layout that shows
    /// only the focused window (`Full`/`Tabbed`), the workspace is retiled
    /// so the newly focused window becomes the visible one.
    pub fn focus_window(&mut self, window: &Window) {
        self.selected_window = Some(window.clone());
        // Follow the *visible* copy of the window: a copied window may also
        // live on hidden workspaces.
        if let Some((_, monitor)) = self.visible_workspace_of_window(window) {
            self.active_monitor = monitor;
        }

        let serial = SERIAL_COUNTER.next_serial();
        if let Some(keyboard) = self.seat.get_keyboard() {
            let surface = window
                .toplevel()
                .map(|toplevel| toplevel.wl_surface().clone());
            keyboard.set_focus(self, surface, serial);
        }

        self.space.raise_element(window, true);
        // The floating layer stays above tiled windows: re-raise the mapped
        // floats after raising a tiled window. Iterate in the space's current
        // stacking order so the floats keep their relative z-order.
        if !self.floating.contains_key(window) {
            let floats: Vec<Window> = self
                .space
                .elements()
                .filter(|element| self.floating.contains_key(*element))
                .cloned()
                .collect();
            for float in floats {
                self.space.raise_element(&float, false);
            }
        }

        for managed in self.managed_windows() {
            managed.set_activated(managed == window);
            if let Some(toplevel) = managed.toplevel() {
                toplevel.send_pending_configure();
            }
        }

        // In a focus-only layout the newly focused window must be mapped in
        // place of the previous one. Check the workspace the window is
        // actually visible on (a copy may also live on hidden workspaces).
        let retile = self
            .visible_workspace_of_window(window)
            .map(|(index, _)| index)
            .or_else(|| self.workspace_index_of_window(window))
            .and_then(|index| self.workspaces.get(index))
            .is_some_and(|workspace| workspace.layout.only_focused_visible());
        if retile {
            self.relayout();
        }
    }

    /// Clears keyboard focus and the selection, deactivating every window.
    ///
    /// Used when switching to an empty workspace or clicking the background.
    pub(crate) fn clear_focus(&mut self) {
        self.selected_window = None;
        for window in self.managed_windows() {
            window.set_activated(false);
            if let Some(toplevel) = window.toplevel() {
                toplevel.send_pending_configure();
            }
        }
        let serial = SERIAL_COUNTER.next_serial();
        if let Some(keyboard) = self.seat.get_keyboard() {
            keyboard.set_focus(self, Option::<WlSurface>::None, serial);
        }
    }

    /// Focuses the master window of workspace `index`, or clears focus when it
    /// is empty.
    pub(crate) fn focus_workspace_master(&mut self, index: usize) {
        if let Some(master) = self
            .workspaces
            .get(index)
            .and_then(|workspace| workspace.windows.first())
            .cloned()
        {
            self.focus_window(&master);
        } else {
            self.clear_focus();
        }
    }

    /// Index of the focused window within `windows`, or 0 if it is not present.
    pub(crate) fn focus_index_within(&self, windows: &[Window]) -> usize {
        self.focused_window()
            .and_then(|focused| windows.iter().position(|w| *w == focused))
            .unwrap_or(0)
    }

    /// Index of the focused window within the current workspace, or 0 if none.
    fn focused_index(&self) -> usize {
        let Some(workspace) = self
            .current_workspace()
            .and_then(|i| self.workspaces.get(i))
        else {
            return 0;
        };
        self.focus_index_within(&workspace.windows)
    }

    /// Focuses the window `count` steps after the focused one on the current
    /// workspace (wrapping).
    fn focus_step(&mut self, step: usize) {
        let Some(workspace) = self
            .current_workspace()
            .and_then(|i| self.workspaces.get(i))
        else {
            return;
        };
        let count = workspace.windows.len();
        if count == 0 {
            return;
        }
        let next = (self.focused_index() + step) % count;
        let window = workspace.windows[next].clone();
        self.focus_window(&window);
    }

    /// Focuses the next window in tiling order on the current workspace.
    pub fn focus_next(&mut self) {
        self.focus_step(1);
    }

    /// Focuses the previous window in tiling order on the current workspace.
    pub fn focus_prev(&mut self) {
        let Some(count) = self
            .current_workspace()
            .and_then(|i| self.workspaces.get(i))
            .map(|workspace| workspace.windows.len())
        else {
            return;
        };
        if count == 0 {
            return;
        }
        // Stepping back by one == stepping forward by count - 1.
        self.focus_step(count - 1);
    }

    /// Swaps the focused window with the master of its workspace, keeping
    /// focus.
    pub fn swap_master(&mut self) {
        let Some(focused) = self.focused_window() else {
            return;
        };
        let Some(workspace) = self
            .current_workspace()
            .and_then(|i| self.workspaces.get_mut(i))
        else {
            return;
        };
        let Some(index) = workspace.windows.iter().position(|w| *w == focused) else {
            return;
        };

        if index != 0 {
            workspace.windows.swap(0, index);
        }
        let master = workspace.windows[0].clone();
        self.relayout();
        self.focus_window(&master);
    }

    /// Moves the focused window one place down the stack, wrapping to the top
    /// (xmonad's `swapDown`), keeping focus on the moved window.
    pub fn swap_down(&mut self) {
        self.swap_focused(SwapDirection::Down);
    }

    /// Moves the focused window one place up the stack, wrapping to the bottom
    /// (xmonad's `swapUp`), keeping focus on the moved window.
    pub fn swap_up(&mut self) {
        self.swap_focused(SwapDirection::Up);
    }

    fn swap_focused(&mut self, direction: SwapDirection) {
        let Some(focused) = self.focused_window() else {
            return;
        };
        let Some(workspace) = self
            .current_workspace()
            .and_then(|i| self.workspaces.get_mut(i))
        else {
            return;
        };
        let Some(index) = workspace.windows.iter().position(|w| *w == focused) else {
            return;
        };
        let len = workspace.windows.len();
        if len < 2 {
            return;
        }

        match direction {
            SwapDirection::Down => {
                if index + 1 < len {
                    workspace.windows.swap(index, index + 1);
                } else {
                    // Focused is last: move it to the front.
                    let window = workspace.windows.remove(index);
                    workspace.windows.insert(0, window);
                }
            }
            SwapDirection::Up => {
                if index >= 1 {
                    workspace.windows.swap(index, index - 1);
                } else {
                    // Focused is first: move it to the back.
                    let window = workspace.windows.remove(0);
                    workspace.windows.push(window);
                }
            }
        }

        self.relayout();
        self.focus_window(&focused);
    }

    /// Shows workspace `target` on the active monitor.
    ///
    /// Follows xmonad's `greedyView`: if `target` is already shown on another
    /// monitor, the two monitors exchange workspaces; otherwise the hidden
    /// `target` replaces whatever the active monitor was showing.
    pub fn view_workspace(&mut self, target: usize) {
        if target >= self.workspaces.len() {
            return;
        }
        let Some(current) = self.current_workspace() else {
            return;
        };
        if current == target {
            return;
        }

        self.record_workspace_history(current);

        if let Some(other) = self.monitor_showing(target) {
            self.monitors[other].workspace = current;
        }
        self.monitors[self.active_monitor].workspace = target;

        self.relayout();
        self.focus_workspace_master(target);
    }

    /// Remembers `workspace` as the most recently left workspace for
    /// [`Self::toggle_workspace`].
    fn record_workspace_history(&mut self, workspace: usize) {
        // The history only needs to cover a handful of recent workspaces.
        const HISTORY_LIMIT: usize = 32;
        self.workspace_history.retain(|&entry| entry != workspace);
        self.workspace_history.push(workspace);
        if self.workspace_history.len() > HISTORY_LIMIT {
            self.workspace_history.remove(0);
        }
    }

    /// Toggles back to the most recently hidden workspace, like xmonad's
    /// `toggleWS` from `XMonad.Actions.CycleWS`.
    ///
    /// Workspaces currently visible on some monitor are skipped, matching
    /// `toggleWS`'s "hidden workspaces only" behavior.
    pub fn toggle_workspace(&mut self) {
        let Some(current) = self.current_workspace() else {
            return;
        };
        let target = self.workspace_history.iter().rev().copied().find(|&i| {
            i != current && i < self.workspaces.len() && self.monitor_showing(i).is_none()
        });
        if let Some(target) = target {
            self.view_workspace(target);
        }
    }

    /// Monitor indices ordered by physical position: top-to-bottom, then
    /// left-to-right — xmonad's default `ScreenComparator`
    /// (`verticalScreenOrderer` compares by `(y, x)`).
    fn physical_monitor_order(&self) -> Vec<usize> {
        let mut order: Vec<usize> = (0..self.monitors.len()).collect();
        order.sort_by_key(|&index| {
            self.space
                .output_geometry(&self.monitors[index].output)
                .map_or((0, 0), |geo| (geo.loc.y, geo.loc.x))
        });
        order
    }

    /// The monitor `offset` steps from the active one in physical order
    /// (wrapping), or `None` with fewer than two monitors.
    fn neighbour_monitor(&self, offset: isize) -> Option<usize> {
        let order = self.physical_monitor_order();
        if order.len() < 2 {
            return None;
        }
        let position = order.iter().position(|&i| i == self.active_monitor)?;
        let len = isize::try_from(order.len()).ok()?;
        let neighbour = (isize::try_from(position).ok()? + offset).rem_euclid(len);
        order.get(usize::try_from(neighbour).ok()?).copied()
    }

    /// Moves focus to the next/previous monitor in physical order, like
    /// `onNextNeighbour`/`onPrevNeighbour W.view` from
    /// `XMonad.Actions.PhysicalScreens`.
    pub fn focus_neighbour_monitor(&mut self, offset: isize) {
        let Some(target) = self.neighbour_monitor(offset) else {
            return;
        };
        self.active_monitor = target;
        let workspace = self.monitors[target].workspace;
        self.focus_workspace_master(workspace);
    }

    /// Moves the focused window to the workspace shown on the next/previous
    /// monitor without following it, like `onNextNeighbour W.shift`.
    pub fn shift_focused_to_neighbour_monitor(&mut self, offset: isize) {
        let Some(target_monitor) = self.neighbour_monitor(offset) else {
            return;
        };
        let target = self.monitors[target_monitor].workspace;
        self.move_focused_to_workspace(target);
    }

    /// Swaps the visible workspaces of the active monitor and its next
    /// neighbour, keeping focus on the active monitor (xmonad's
    /// `swapNextScreen`).
    pub fn swap_next_screen(&mut self) {
        let Some(next) = self.neighbour_monitor(1) else {
            return;
        };
        let current = self.monitors[self.active_monitor].workspace;
        self.record_workspace_history(current);
        self.monitors[self.active_monitor].workspace = self.monitors[next].workspace;
        self.monitors[next].workspace = current;

        self.relayout();
        let workspace = self.monitors[self.active_monitor].workspace;
        self.focus_workspace_master(workspace);
    }

    /// Moves the focused window to workspace `target` without switching view,
    /// matching xmonad's `mod+Shift+N`.
    pub fn move_focused_to_workspace(&mut self, target: usize) {
        if target >= self.workspaces.len() {
            return;
        }
        let Some(focused) = self.focused_window() else {
            return;
        };
        let Some(source) = self.source_workspace_of_window(&focused) else {
            return;
        };
        if source == target {
            return;
        }

        self.workspaces[source].windows.retain(|w| *w != focused);
        // A copy may already live on the target workspace.
        if !self.workspaces[target].windows.contains(&focused) {
            self.workspaces[target].windows.insert(0, focused.clone());
        }

        self.relayout();
        // Focus stays on the current workspace; pick its new master.
        if let Some(current) = self.current_workspace() {
            self.focus_workspace_master(current);
        }
    }

    /// Moves the focused window to the workspace shown on the next monitor
    /// (wrapping) and follows it there.
    pub fn move_focused_to_next_monitor(&mut self) {
        if self.monitors.len() < 2 {
            return;
        }
        let Some(focused) = self.focused_window() else {
            return;
        };
        let Some(source) = self.source_workspace_of_window(&focused) else {
            return;
        };
        let next = (self.active_monitor + 1) % self.monitors.len();
        let target = self.monitors[next].workspace;
        if source == target {
            return;
        }

        self.workspaces[source].windows.retain(|w| *w != focused);
        // A copy may already live on the target workspace.
        if !self.workspaces[target].windows.contains(&focused) {
            self.workspaces[target].windows.insert(0, focused.clone());
        }
        self.active_monitor = next;
        self.relayout();
        self.focus_window(&focused);
    }

    /// Renames the current workspace, e.g. for a user-defined named workspace.
    pub fn rename_current_workspace(&mut self, name: String) {
        if let Some(workspace) = self
            .current_workspace()
            .and_then(|i| self.workspaces.get_mut(i))
        {
            workspace.name = name;
        }
    }

    /// Requests that the currently focused window closes.
    pub fn close_focused_window(&self) {
        let Some(window) = self.focused_window() else {
            return;
        };
        if let Some(toplevel) = window.toplevel() {
            toplevel.send_close();
        }
    }

    /// Advances the current workspace to the next layout in its ring, keeping
    /// each layout's state (xmonad's `NextLayout`, mod+space).
    pub fn next_layout(&mut self) {
        if let Some(workspace) = self
            .current_workspace()
            .and_then(|i| self.workspaces.get_mut(i))
        {
            workspace.layout.next();
        }
        self.relayout();
    }

    /// Resets the current workspace's layout ring to its defaults (xmonad's
    /// `ResetLayout`, mod+shift+space).
    pub fn reset_layout(&mut self) {
        if let Some(workspace) = self
            .current_workspace()
            .and_then(|i| self.workspaces.get_mut(i))
        {
            workspace.layout.reset();
        }
        self.relayout();
    }

    /// Increases the master count of the current layout (xmonad's `IncMasterN
    /// 1`, mod+comma).
    pub fn inc_master(&mut self) {
        self.change_master(1);
    }

    /// Decreases the master count of the current layout (xmonad's `IncMasterN
    /// (-1)`, mod+period).
    pub fn dec_master(&mut self) {
        self.change_master(-1);
    }

    fn change_master(&mut self, delta: i32) {
        if let Some(workspace) = self
            .current_workspace()
            .and_then(|i| self.workspaces.get_mut(i))
        {
            workspace.layout.inc_master(delta);
        }
        self.relayout();
    }

    /// Expands the focused window vertically within its layout column.
    pub fn expand_focused_window(&mut self) {
        let index = self.focused_index();
        if let Some(workspace) = self
            .current_workspace()
            .and_then(|i| self.workspaces.get_mut(i))
        {
            let count = workspace.windows.len();
            workspace.layout.expand_focused(index, count);
        }
        self.relayout();
    }

    /// Shrinks the focused window vertically within its layout column.
    pub fn shrink_focused_window(&mut self) {
        let index = self.focused_index();
        if let Some(workspace) = self
            .current_workspace()
            .and_then(|i| self.workspaces.get_mut(i))
        {
            let count = workspace.windows.len();
            workspace.layout.shrink_focused(index, count);
        }
        self.relayout();
    }

    /// Expands the current workspace's master area horizontally.
    pub fn expand_master_area(&mut self) {
        if let Some(workspace) = self
            .current_workspace()
            .and_then(|i| self.workspaces.get_mut(i))
        {
            workspace.layout.expand_master();
        }
        self.relayout();
    }

    /// Shrinks the current workspace's master area horizontally.
    pub fn shrink_master_area(&mut self) {
        if let Some(workspace) = self
            .current_workspace()
            .and_then(|i| self.workspaces.get_mut(i))
        {
            workspace.layout.shrink_master();
        }
        self.relayout();
    }

    /// Builds the border render elements for every visible managed window,
    /// skipping workspaces whose current layout draws no borders (`Full`).
    ///
    /// Only workspaces shown on a monitor are considered, so a window copied
    /// onto a hidden workspace neither gets a duplicate border nor sneaks a
    /// border past a visible `noBorders Full` workspace.
    #[must_use]
    pub fn border_elements(&self, scale: f64) -> Vec<SolidColorRenderElement> {
        let border = config::BORDER_WIDTH;
        let focused = self.focused_window();

        let mut elements = Vec::new();
        let mut seen: Vec<&Window> = Vec::new();
        for monitor in &self.monitors {
            let Some(workspace) = self.workspaces.get(monitor.workspace) else {
                continue;
            };
            if workspace.layout.is_full() {
                continue;
            }
            for window in &workspace.windows {
                if seen.contains(&window) {
                    continue;
                }
                seen.push(window);
                // Unmapped windows (e.g. hidden `Tabbed` ones) have no
                // geometry.
                let Some(geometry) = self.space.element_geometry(window) else {
                    continue;
                };
                let color = if focused.as_ref() == Some(window) {
                    config::BORDER_COLOR_FOCUSED
                } else {
                    config::BORDER_COLOR_NORMAL
                };

                for ring in border_ring(geometry, border) {
                    elements.push(solid_element(ring, color, scale));
                }
            }
        }

        elements
    }
}

/// Which way [`Oxmonad::swap_focused`] moves the focused window in the stack.
#[derive(Debug, Clone, Copy)]
enum SwapDirection {
    /// Toward the end of the stack, wrapping to the front.
    Down,
    /// Toward the front of the stack, wrapping to the end.
    Up,
}

/// Insets a tile by `border` on every side to get the window's content
/// geometry; the border ring is drawn in the gap around it.
fn window_geometry(tile: Rectangle<i32, Logical>, border: i32) -> Rectangle<i32, Logical> {
    Rectangle::new(
        Point::from((tile.loc.x + border, tile.loc.y + border)),
        Size::from((
            (tile.size.w - 2 * border).max(1),
            (tile.size.h - 2 * border).max(1),
        )),
    )
}

/// The four rectangles forming a border ring just outside `geometry`.
fn border_ring(geometry: Rectangle<i32, Logical>, border: i32) -> [Rectangle<i32, Logical>; 4] {
    let Rectangle { loc, size } = geometry;
    [
        // Top.
        Rectangle::new(
            Point::from((loc.x - border, loc.y - border)),
            Size::from((size.w + 2 * border, border)),
        ),
        // Bottom.
        Rectangle::new(
            Point::from((loc.x - border, loc.y + size.h)),
            Size::from((size.w + 2 * border, border)),
        ),
        // Left.
        Rectangle::new(
            Point::from((loc.x - border, loc.y)),
            Size::from((border, size.h)),
        ),
        // Right.
        Rectangle::new(
            Point::from((loc.x + size.w, loc.y)),
            Size::from((border, size.h)),
        ),
    ]
}

/// Builds a solid-color render element filling `rect`.
fn solid_element(
    rect: Rectangle<i32, Logical>,
    color: Color32F,
    scale: f64,
) -> SolidColorRenderElement {
    let buffer = SolidColorBuffer::new(rect.size, color);
    SolidColorRenderElement::from_buffer(
        &buffer,
        rect.loc.to_physical_precise_round(scale),
        scale,
        1.0,
        Kind::Unspecified,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: i32, y: i32, w: i32, h: i32) -> Rectangle<i32, Logical> {
        Rectangle::new(Point::from((x, y)), Size::from((w, h)))
    }

    fn screen() -> Rectangle<i32, Logical> {
        rect(0, 0, 1000, 1000)
    }

    #[test]
    fn no_windows_yields_no_tiles() {
        assert!(ResizableTall::default().arrange(0, screen()).is_empty());
    }

    #[test]
    fn single_window_fills_the_screen() {
        let tiles = ResizableTall::default().arrange(1, screen());
        assert_eq!(tiles, vec![rect(0, 0, 1000, 1000)]);
    }

    #[test]
    fn two_windows_split_master_and_stack() {
        // ratio 3/5 -> master column is 600px wide, stack gets the rest.
        let tiles = ResizableTall::default().arrange(2, screen());
        assert_eq!(tiles, vec![rect(0, 0, 600, 1000), rect(600, 0, 400, 1000)]);
    }

    #[test]
    fn stack_windows_share_the_height() {
        let tiles = ResizableTall::default().arrange(3, screen());
        assert_eq!(
            tiles,
            vec![
                rect(0, 0, 600, 1000),
                rect(600, 0, 400, 500),
                rect(600, 500, 400, 500),
            ]
        );
    }

    #[test]
    fn rows_absorb_the_rounding_remainder() {
        // 1000 / 3 = 333, last row takes the remaining 334.
        let tiles = ResizableTall {
            nmaster: 3,
            ..ResizableTall::default()
        }
        .arrange(3, screen());
        assert_eq!(
            tiles,
            vec![
                rect(0, 0, 1000, 333),
                rect(0, 333, 1000, 333),
                rect(0, 666, 1000, 334),
            ]
        );
    }

    #[test]
    fn expanding_stack_window_gives_it_more_height() {
        let mut layout = ResizableTall::default();
        layout.expand_window(1, 3);

        let tiles = layout.arrange(3, screen());
        assert_eq!(
            tiles,
            vec![
                rect(0, 0, 600, 1000),
                rect(600, 0, 400, 507),
                rect(600, 507, 400, 493),
            ]
        );
    }

    #[test]
    fn shrinking_stack_window_gives_the_other_row_more_height() {
        let mut layout = ResizableTall::default();
        layout.shrink_window(1, 3);

        let tiles = layout.arrange(3, screen());
        assert_eq!(
            tiles,
            vec![
                rect(0, 0, 600, 1000),
                rect(600, 0, 400, 492),
                rect(600, 492, 400, 508),
            ]
        );
    }

    #[test]
    fn expanding_master_area_makes_master_wider() {
        let mut layout = ResizableTall::default();
        layout.expand_master();

        let tiles = layout.arrange(2, screen());
        assert_eq!(tiles, vec![rect(0, 0, 630, 1000), rect(630, 0, 370, 1000)]);
    }

    #[test]
    fn shrinking_master_area_makes_master_narrower() {
        let mut layout = ResizableTall::default();
        layout.shrink_master();

        let tiles = layout.arrange(2, screen());
        assert_eq!(tiles, vec![rect(0, 0, 570, 1000), rect(570, 0, 430, 1000)]);
    }

    #[test]
    fn inc_master_moves_a_window_into_the_master_column() {
        let mut layout = ResizableTall::default();
        layout.inc_master(1);
        // With nmaster == 2 both windows fill the full-width master column.
        let tiles = layout.arrange(2, screen());
        assert_eq!(tiles, vec![rect(0, 0, 1000, 500), rect(0, 500, 1000, 500)]);
    }

    #[test]
    fn dec_master_below_zero_puts_everything_in_the_stack() {
        let mut layout = ResizableTall::default();
        layout.inc_master(-5);
        assert_eq!(layout.nmaster, 0);
        // nmaster == 0: a single full-width column split across all windows.
        let tiles = layout.arrange(2, screen());
        assert_eq!(tiles, vec![rect(0, 0, 1000, 500), rect(0, 500, 1000, 500)]);
    }

    #[test]
    fn reflect_tall_puts_master_on_the_right() {
        let tall = ResizableTall::default();
        let reflected = reflect_horizontally(screen(), tall.arrange(2, screen()));
        // Master (600 wide) reflected to the right edge; stack on the left.
        assert_eq!(
            reflected,
            vec![rect(400, 0, 600, 1000), rect(0, 0, 400, 1000)]
        );
    }

    #[test]
    fn mirror_tall_puts_master_on_top() {
        let tall = ResizableTall::default();
        let mirrored_tiles = mirrored(screen(), |a| tall.arrange(2, a));
        // Master row on top (600 tall), stack row below.
        assert_eq!(
            mirrored_tiles,
            vec![rect(0, 0, 1000, 600), rect(0, 600, 1000, 400)]
        );
    }

    #[test]
    fn full_places_only_the_focused_window() {
        let placed = full_arrange(3, 2, screen());
        assert_eq!(placed, vec![(2, screen())]);
    }

    #[test]
    fn tabbed_reserves_the_strip_and_shows_the_focused_window() {
        let placed = tabbed_arrange(3, 1, screen());
        assert_eq!(
            placed,
            vec![(1, rect(0, TAB_BAR_HEIGHT, 1000, 1000 - TAB_BAR_HEIGHT))]
        );
    }

    #[test]
    fn tabbed_with_one_window_hides_the_strip() {
        let placed = tabbed_arrange(1, 0, screen());
        assert_eq!(placed, vec![(0, screen())]);
    }

    #[test]
    fn grid_four_windows_is_a_two_by_two() {
        let tiles = Grid::default().arrange(4, screen());
        assert_eq!(
            tiles,
            vec![
                rect(0, 0, 500, 500),
                rect(0, 500, 500, 500),
                rect(500, 0, 500, 500),
                rect(500, 500, 500, 500),
            ]
        );
    }

    #[test]
    fn grid_three_windows_fills_columns_then_adds_the_extra_to_the_right() {
        // n=3: ncols=round(sqrt(3*1000/(1000*16/9)))=round(sqrt(1.6875))=round(1.30)=1.
        // Wait: with a wide ratio ncols stays small; verify column-major fill.
        let tiles = Grid::default().arrange(3, screen());
        // ncols=1 -> a single column split into three rows (334/333/333 via chop).
        assert_eq!(
            tiles,
            vec![
                rect(0, 0, 1000, 334),
                rect(0, 334, 1000, 333),
                rect(0, 667, 1000, 333),
            ]
        );
    }

    #[test]
    fn three_col_single_window_fills_the_screen() {
        let tiles = ThreeCol::default().arrange(1, screen());
        assert_eq!(tiles, vec![rect(0, 0, 1000, 1000)]);
    }

    #[test]
    fn three_col_two_windows_split_in_half() {
        // n == nmaster+1: split 50/50, master left, single slave right.
        let tiles = ThreeCol::default().arrange(2, screen());
        assert_eq!(tiles, vec![rect(0, 0, 500, 1000), rect(500, 0, 500, 1000)]);
    }

    #[test]
    fn three_col_three_windows_uses_three_columns() {
        // n > nmaster+1: master r1w=ceil(1000*0.5)=500, r2w=ceil(500/2)=250, r3w=250.
        // nslave=2 -> nslave1=ceil(2/2)=1 in middle, nslave2=1 on the right.
        let tiles = ThreeCol::default().arrange(3, screen());
        assert_eq!(
            tiles,
            vec![
                rect(0, 0, 500, 1000),
                rect(500, 0, 250, 1000),
                rect(750, 0, 250, 1000),
            ]
        );
    }

    #[test]
    fn three_col_four_windows_fills_middle_before_right() {
        // nslave=3 -> nslave1=ceil(3/2)=2 (middle column), nslave2=1 (right).
        let tiles = ThreeCol::default().arrange(4, screen());
        assert_eq!(
            tiles,
            vec![
                rect(0, 0, 500, 1000),
                rect(500, 0, 250, 500),
                rect(500, 500, 250, 500),
                rect(750, 0, 250, 1000),
            ]
        );
    }

    #[test]
    fn layout_cycle_advances_without_resetting_state() {
        let mut cycle = LayoutCycle::default();
        assert_eq!(cycle.name(), "ReflectTall");
        cycle.next();
        assert_eq!(cycle.name(), "Tall");
        // Resize the Tall layout, cycle all the way round, and confirm the
        // state survived (NextLayout keeps each layout's state).
        cycle.expand_master();
        for _ in 0..cycle.layouts.len() {
            cycle.next();
        }
        assert_eq!(cycle.name(), "Tall");
        let tiles = cycle.arrange(2, 0, screen());
        assert_eq!(tiles[0].1, rect(0, 0, 630, 1000));
    }

    #[test]
    fn reset_layout_restores_defaults() {
        let mut cycle = LayoutCycle::default();
        cycle.next();
        cycle.expand_master();
        cycle.reset();
        assert_eq!(cycle.name(), "ReflectTall");
        // The Tall layout's ratio is back to the 3/5 default.
        cycle.next();
        let tiles = cycle.arrange(2, 0, screen());
        assert_eq!(tiles[0].1, rect(0, 0, 600, 1000));
    }

    #[test]
    fn full_layout_ignores_master_resize() {
        let mut full = Layout::Full;
        full.expand_master();
        // No state to change; arrange still fullscreens the focused window.
        assert_eq!(full.arrange(2, 1, screen()), vec![(1, screen())]);
    }
}
