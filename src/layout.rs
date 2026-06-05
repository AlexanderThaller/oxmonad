//! Window tiling, focus management and border rendering.
//!
//! The only layout for now is [`ResizableTall`], a master/stack tiler modelled
//! on xmonad's `XMonad.Layout.ResizableTile`: `nmaster` windows fill a master
//! column on the left occupying `ratio` of the width, and the rest share a
//! stack column on the right. Focused windows can be resized vertically within
//! their current column, matching the `MirrorExpand`/`MirrorShrink` behavior
//! from the user's xmonad config.

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

/// One monitor: an output plus the windows tiled on it and their layout.
///
/// Each monitor tiles independently, xmonad-style — windows belong to exactly
/// one monitor and never spill onto another. New windows open on the
/// [active monitor](Oxmonad::active_monitor), and can be sent to another
/// monitor with [`Oxmonad::move_focused_to_next_monitor`].
#[derive(Debug)]
pub struct Monitor {
    /// The output this monitor renders to.
    pub output: Output,
    /// Windows tiled on this monitor in tiling order; index 0 is the master.
    pub windows: Vec<Window>,
    /// The tiling layout used to arrange this monitor's windows.
    pub layout: ResizableTall,
}

impl Monitor {
    fn new(output: Output) -> Self {
        Self {
            output,
            windows: Vec::new(),
            layout: ResizableTall::default(),
        }
    }
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

        // The master column takes the full width when there is no stack.
        let master_width = if n_stack == 0 {
            area.size.w
        } else {
            #[expect(
                clippy::cast_possible_truncation,
                reason = "Master width is a screen-bounded, non-negative pixel count."
            )]
            let width = (f64::from(area.size.w) * self.ratio).round() as i32;
            width
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

/// Splits a column into `count` equal-height rows, pushing them onto `out`.
///
/// The last row absorbs any rounding remainder so the rows exactly fill the
/// column height.
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
            #[expect(
                clippy::cast_possible_truncation,
                reason = "Row height is a screen-bounded, non-negative pixel count."
            )]
            let height = (f64::from(size.h) * weight / total_weight).round() as i32;
            height
        };
        out.push(Rectangle::new(
            Point::from((loc.x, y)),
            Size::from((size.w, height)),
        ));
        y += height;
        used_height += height;
    }
}

impl Oxmonad {
    /// Registers a freshly-detected `output` as a new monitor.
    pub fn add_monitor(&mut self, output: Output) {
        self.monitors.push(Monitor::new(output));
    }

    /// Iterates every managed window across all monitors.
    pub fn managed_windows(&self) -> impl Iterator<Item = &Window> {
        self.monitors
            .iter()
            .flat_map(|monitor| monitor.windows.iter())
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

    /// Index into [`Self::monitors`] of the monitor holding `window`, if any.
    fn monitor_index_of_window(&self, window: &Window) -> Option<usize> {
        self.monitors
            .iter()
            .position(|monitor| monitor.windows.contains(window))
    }

    /// Recomputes the layout and repositions/resizes every managed window.
    ///
    /// Each monitor tiles its own windows within its output, offset into global
    /// space; only the primary monitor reserves room for the built-in bar.
    pub fn relayout(&mut self) {
        for monitor in &mut self.monitors {
            monitor.windows.retain(IsAlive::alive);
        }
        if self
            .selected_window
            .as_ref()
            .is_some_and(|window| !window.alive())
        {
            self.selected_window = None;
        }

        let border = config::BORDER_WIDTH;

        // Collect placements first so we don't borrow the monitors while
        // mutating `self.space`.
        let mut placements: Vec<Placement> = Vec::new();
        for monitor in &self.monitors {
            let Some(output_geo) = self.space.output_geometry(&monitor.output) else {
                continue;
            };
            // `non_exclusive_zone` is output-local; shift it into global space.
            let mut area = layer_map_for_output(&monitor.output).non_exclusive_zone();
            area.loc += output_geo.loc;
            // Only the primary monitor gives up a strip for the built-in bar.
            let area = if self.is_primary_output(&monitor.output) {
                crate::bar::content_area(area)
            } else {
                area
            };

            let tiles = monitor.layout.arrange(monitor.windows.len(), area);
            for (window, tile) in monitor.windows.iter().cloned().zip(tiles) {
                let geometry = window_geometry(tile, border);
                placements.push((window, geometry.loc, geometry.size));
            }
        }

        for (window, loc, size) in placements {
            if let Some(toplevel) = window.toplevel() {
                toplevel.with_pending_state(|state| state.size = Some(size));
                toplevel.send_pending_configure();
            }
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
    /// Focus follows the monitor: the window's monitor becomes active.
    pub fn focus_window(&mut self, window: &Window) {
        self.selected_window = Some(window.clone());
        if let Some(index) = self.monitor_index_of_window(window) {
            self.active_monitor = index;
        }

        let serial = SERIAL_COUNTER.next_serial();
        if let Some(keyboard) = self.seat.get_keyboard() {
            let surface = window
                .toplevel()
                .map(|toplevel| toplevel.wl_surface().clone());
            keyboard.set_focus(self, surface, serial);
        }

        self.space.raise_element(window, true);

        for managed in self.managed_windows() {
            managed.set_activated(managed == window);
            if let Some(toplevel) = managed.toplevel() {
                toplevel.send_pending_configure();
            }
        }
    }

    /// Index of the focused window within the active monitor, or 0 when none.
    fn focused_index(&self) -> usize {
        let Some(monitor) = self.monitors.get(self.active_monitor) else {
            return 0;
        };
        self.focused_window()
            .and_then(|focused| monitor.windows.iter().position(|w| *w == focused))
            .unwrap_or(0)
    }

    /// Focuses the window `count` steps after the focused one on the active
    /// monitor (wrapping).
    fn focus_step(&mut self, step: usize) {
        let Some(monitor) = self.monitors.get(self.active_monitor) else {
            return;
        };
        let count = monitor.windows.len();
        if count == 0 {
            return;
        }
        let next = (self.focused_index() + step) % count;
        let window = monitor.windows[next].clone();
        self.focus_window(&window);
    }

    /// Focuses the next window in tiling order on the active monitor.
    pub fn focus_next(&mut self) {
        self.focus_step(1);
    }

    /// Focuses the previous window in tiling order on the active monitor.
    pub fn focus_prev(&mut self) {
        let Some(count) = self
            .monitors
            .get(self.active_monitor)
            .map(|m| m.windows.len())
        else {
            return;
        };
        if count == 0 {
            return;
        }
        // Stepping back by one == stepping forward by count - 1.
        self.focus_step(count - 1);
    }

    /// Swaps the focused window with the master window of its monitor and keeps
    /// focus on it.
    pub fn swap_master(&mut self) {
        let Some(focused) = self.focused_window() else {
            return;
        };
        let Some(monitor) = self.monitors.get_mut(self.active_monitor) else {
            return;
        };
        let Some(index) = monitor.windows.iter().position(|w| *w == focused) else {
            return;
        };

        if index != 0 {
            monitor.windows.swap(0, index);
        }
        let master = monitor.windows[0].clone();
        self.relayout();
        self.focus_window(&master);
    }

    /// Moves the focused window to the next monitor (wrapping) and follows it.
    pub fn move_focused_to_next_monitor(&mut self) {
        if self.monitors.len() < 2 {
            return;
        }
        let Some(focused) = self.focused_window() else {
            return;
        };
        let Some(from) = self.monitor_index_of_window(&focused) else {
            return;
        };
        let to = (from + 1) % self.monitors.len();

        if let Some(monitor) = self.monitors.get_mut(from) {
            monitor.windows.retain(|w| *w != focused);
        }
        // Land as the master of the destination monitor, like a fresh window.
        if let Some(monitor) = self.monitors.get_mut(to) {
            monitor.windows.insert(0, focused.clone());
        }
        self.active_monitor = to;
        self.relayout();
        self.focus_window(&focused);
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

    /// Expands the focused window vertically within its layout column.
    pub fn expand_focused_window(&mut self) {
        let index = self.focused_index();
        if let Some(monitor) = self.monitors.get_mut(self.active_monitor) {
            let count = monitor.windows.len();
            monitor.layout.expand_window(index, count);
        }
        self.relayout();
    }

    /// Shrinks the focused window vertically within its layout column.
    pub fn shrink_focused_window(&mut self) {
        let index = self.focused_index();
        if let Some(monitor) = self.monitors.get_mut(self.active_monitor) {
            let count = monitor.windows.len();
            monitor.layout.shrink_window(index, count);
        }
        self.relayout();
    }

    /// Expands the active monitor's master area horizontally.
    pub fn expand_master_area(&mut self) {
        if let Some(monitor) = self.monitors.get_mut(self.active_monitor) {
            monitor.layout.expand_master();
        }
        self.relayout();
    }

    /// Shrinks the active monitor's master area horizontally.
    pub fn shrink_master_area(&mut self) {
        if let Some(monitor) = self.monitors.get_mut(self.active_monitor) {
            monitor.layout.shrink_master();
        }
        self.relayout();
    }

    /// Builds the border render elements for every managed window.
    #[must_use]
    pub fn border_elements(&self, scale: f64) -> Vec<SolidColorRenderElement> {
        let border = config::BORDER_WIDTH;
        let focused = self.focused_window();

        let mut elements = Vec::new();
        for window in self.managed_windows() {
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

        elements
    }
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
}
