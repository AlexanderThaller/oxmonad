//! Window tiling, focus management and border rendering.
//!
//! The only layout for now is [`ResizableTall`], a master/stack tiler modelled
//! on xmonad's `XMonad.Layout.ResizableTile`: `nmaster` windows fill a master
//! column on the left occupying `ratio` of the width, and the rest share a
//! stack column on the right. (The per-window resizing that the "Resizable"
//! name refers to is not wired up yet — see the keybindings TODO.)

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

/// A master/stack tiling layout (xmonad's `ResizableTall`).
#[derive(Debug, Clone, Copy)]
pub struct ResizableTall {
    /// Number of windows in the master column.
    pub nmaster: usize,
    /// Fraction of the screen width occupied by the master column.
    pub ratio: f64,
    /// Amount to change `ratio` by when resizing (not used yet).
    pub delta: f64,
}

impl Default for ResizableTall {
    fn default() -> Self {
        // Matches the user's xmonad config: ResizableTall 1 (3/100) (3/5).
        Self {
            nmaster: 1,
            ratio: 3.0 / 5.0,
            delta: 3.0 / 100.0,
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
            nmaster,
            &mut tiles,
        );

        if n_stack > 0 {
            split_column(
                Point::from((area.loc.x + master_width, area.loc.y)),
                Size::from((area.size.w - master_width, area.size.h)),
                n_stack,
                &mut tiles,
            );
        }

        tiles
    }
}

/// Splits a column into `count` equal-height rows, pushing them onto `out`.
///
/// The last row absorbs any rounding remainder so the rows exactly fill the
/// column height.
fn split_column(
    loc: Point<i32, Logical>,
    size: Size<i32, Logical>,
    count: usize,
    out: &mut Vec<Rectangle<i32, Logical>>,
) {
    debug_assert!(count > 0);
    // Window counts are tiny; saturate rather than risk a panic on conversion.
    let count = i32::try_from(count).unwrap_or(i32::MAX);
    let row_height = size.h / count;
    for i in 0..count {
        let y = loc.y + row_height * i;
        let height = if i == count - 1 {
            size.h - row_height * (count - 1)
        } else {
            row_height
        };
        out.push(Rectangle::new(
            Point::from((loc.x, y)),
            Size::from((size.w, height)),
        ));
    }
}

impl Oxmonad {
    /// Recomputes the layout and repositions/resizes every managed window.
    pub fn relayout(&mut self) {
        self.windows.retain(IsAlive::alive);

        let Some(output) = self.space.outputs().next().cloned() else {
            return;
        };
        // Tile within the area left after layer-shell exclusive zones (bars
        // etc.); for an overlay launcher this is the whole output.
        let area = layer_map_for_output(&output).non_exclusive_zone();

        let tiles = self.layout.arrange(self.windows.len(), area);
        let border = config::BORDER_WIDTH;

        // Collect placements first so we don't borrow `self.windows` while
        // mutating `self.space`.
        let placements: Vec<Placement> = self
            .windows
            .iter()
            .cloned()
            .zip(tiles)
            .map(|(window, tile)| {
                let geometry = window_geometry(tile, border);
                (window, geometry.loc, geometry.size)
            })
            .collect();

        for (window, loc, size) in placements {
            if let Some(toplevel) = window.toplevel() {
                toplevel.with_pending_state(|state| state.size = Some(size));
                toplevel.send_pending_configure();
            }
            self.space.map_element(window, loc, false);
        }
    }

    /// Returns the currently keyboard-focused managed window, if any.
    #[must_use]
    pub fn focused_window(&self) -> Option<Window> {
        let focus = self.seat.get_keyboard()?.current_focus()?;
        self.windows
            .iter()
            .find(|window| {
                window
                    .toplevel()
                    .is_some_and(|toplevel| toplevel.wl_surface() == &focus)
            })
            .cloned()
    }

    /// Gives keyboard focus to `window`, raising it and updating activation.
    pub fn focus_window(&mut self, window: &Window) {
        let serial = SERIAL_COUNTER.next_serial();
        if let Some(keyboard) = self.seat.get_keyboard() {
            let surface = window
                .toplevel()
                .map(|toplevel| toplevel.wl_surface().clone());
            keyboard.set_focus(self, surface, serial);
        }

        self.space.raise_element(window, true);

        for managed in &self.windows {
            managed.set_activated(managed == window);
            if let Some(toplevel) = managed.toplevel() {
                toplevel.send_pending_configure();
            }
        }
    }

    /// Index of the focused window in tiling order, or 0 when none is focused.
    fn focused_index(&self) -> usize {
        self.focused_window()
            .and_then(|focused| self.windows.iter().position(|w| *w == focused))
            .unwrap_or(0)
    }

    /// Focuses the window `count` steps after the focused one (wrapping).
    fn focus_step(&mut self, step: usize) {
        let count = self.windows.len();
        if count == 0 {
            return;
        }
        let next = (self.focused_index() + step) % count;
        let window = self.windows[next].clone();
        self.focus_window(&window);
    }

    /// Focuses the next window in tiling order.
    pub fn focus_next(&mut self) {
        self.focus_step(1);
    }

    /// Focuses the previous window in tiling order.
    pub fn focus_prev(&mut self) {
        let count = self.windows.len();
        if count == 0 {
            return;
        }
        // Stepping back by one == stepping forward by count - 1.
        self.focus_step(count - 1);
    }

    /// Swaps the focused window with the master window and keeps focus on it.
    pub fn swap_master(&mut self) {
        let Some(focused) = self.focused_window() else {
            return;
        };
        let Some(index) = self.windows.iter().position(|w| *w == focused) else {
            return;
        };

        if index != 0 {
            self.windows.swap(0, index);
        }
        self.relayout();

        let master = self.windows[0].clone();
        self.focus_window(&master);
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

    /// Builds the border render elements for every managed window.
    #[must_use]
    pub fn border_elements(&self, scale: f64) -> Vec<SolidColorRenderElement> {
        let border = config::BORDER_WIDTH;
        let focused = self.focused_window();

        let mut elements = Vec::with_capacity(self.windows.len() * 4);
        for window in &self.windows {
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

    /// Builds the render elements for a tiled layout of `n` mock windows with
    /// window `focused` highlighted, reusing the real layout and border code.
    fn tiled_layout_elements(
        layout: ResizableTall,
        n: usize,
        focused: usize,
        area: Rectangle<i32, Logical>,
    ) -> Vec<SolidColorRenderElement> {
        // Stand-in for real client content, so the snapshot stays deterministic.
        const MOCK_WINDOW: Color32F = Color32F::new(0.15, 0.15, 0.18, 1.0);

        let border = config::BORDER_WIDTH;
        let mut elements = Vec::new();
        for (i, tile) in layout.arrange(n, area).into_iter().enumerate() {
            let geometry = window_geometry(tile, border);
            let color = if i == focused {
                config::BORDER_COLOR_FOCUSED
            } else {
                config::BORDER_COLOR_NORMAL
            };
            // Border ring first (drawn on top), then the mock window fill.
            for ring in border_ring(geometry, border) {
                elements.push(solid_element(ring, color, 1.0));
            }
            elements.push(solid_element(geometry, MOCK_WINDOW, 1.0));
        }
        elements
    }

    #[test]
    fn snapshot_single_window() {
        let elements = tiled_layout_elements(ResizableTall::default(), 1, 0, screen());
        crate::snapshot::assert_snapshot("single_window", 1000, 1000, &elements);
    }

    #[test]
    fn snapshot_two_windows_master_focused() {
        let elements = tiled_layout_elements(ResizableTall::default(), 2, 0, screen());
        crate::snapshot::assert_snapshot("two_windows_master_focused", 1000, 1000, &elements);
    }

    #[test]
    fn snapshot_three_windows_stack_focused() {
        let elements = tiled_layout_elements(ResizableTall::default(), 3, 1, screen());
        crate::snapshot::assert_snapshot("three_windows_stack_focused", 1000, 1000, &elements);
    }
}
