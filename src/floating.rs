//! Floating window support, like xmonad's `W.floating` layer.
//!
//! A floating window is skipped by tiling and keeps its own geometry, stored
//! in [`Oxmonad::floating`]. Windows start floating when a manage rule
//! ([`crate::rules`]) says so or when the user drags them (a move grab floats
//! its window, like xmonad's `mod+drag`). `META + t` sinks the focused window
//! back into the tiled layer.

use smithay::{
    desktop::Window,
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{
        Logical,
        Point,
        Rectangle,
        Size,
    },
    wayland::{
        compositor::with_states,
        shell::xdg::XdgToplevelSurfaceData,
    },
};

use crate::{
    Oxmonad,
    rules::{
        self,
        ManageDecision,
    },
};

impl Oxmonad {
    /// Whether `window` is in the floating layer.
    #[must_use]
    pub fn is_floating(&self, window: &Window) -> bool {
        self.floating.contains_key(window)
    }

    /// Floats `window` at `geometry`, raising it above the tiled layer.
    pub fn float_window(&mut self, window: &Window, geometry: Rectangle<i32, Logical>) {
        self.floating.insert(window.clone(), geometry);
        self.relayout();
    }

    /// Records that a grab moved a floating `window` to `location`.
    pub fn update_floating_location(&mut self, window: &Window, location: Point<i32, Logical>) {
        if let Some(geometry) = self.floating.get_mut(window) {
            geometry.loc = location;
        }
    }

    /// Sinks the focused window back into the tiled layer (`META + t`,
    /// xmonad's `withFocused $ windows . W.sink`).
    pub fn sink_focused_window(&mut self) {
        let Some(focused) = self.focused_window() else {
            return;
        };
        if self.floating.remove(&focused).is_some() {
            self.relayout();
        }
    }

    /// Applies the manage rules to a pending window once its metadata is
    /// available, on commit of its toplevel `surface`.
    ///
    /// Rules run exactly once per window, as soon as the client has provided
    /// a title or app id (or on the first commit after that, for clients that
    /// never set either).
    pub fn apply_pending_manage(&mut self, surface: &WlSurface) {
        let Some(position) = self.pending_manage.iter().position(|window| {
            window
                .toplevel()
                .is_some_and(|toplevel| toplevel.wl_surface() == surface)
        }) else {
            return;
        };

        let (title, app_id) = surface_meta(surface);
        if title.is_none() && app_id.is_none() {
            // Nothing to match on yet; wait for a later commit. The window
            // stays tiled in the meantime.
            return;
        }
        let window = self.pending_manage.remove(position);

        match rules::manage_decision(app_id.as_deref(), title.as_deref()) {
            ManageDecision::Tile => {}
            ManageDecision::Float => {
                let geometry = self.centered_float_geometry(&window);
                self.float_window(&window, geometry);
            }
            ManageDecision::SinkSwapDown => {
                self.floating.remove(&window);
                self.swap_new_master_down(&window);
                self.relayout();
            }
        }
    }

    /// Drops a destroyed window from the floating layer and the pending
    /// manage queue.
    pub fn forget_window(&mut self, window: &Window) {
        self.floating.remove(window);
        self.pending_manage.retain(|pending| pending != window);
    }

    /// A centered rectangle for floating `window` on the monitor showing its
    /// workspace (falling back to the active monitor), using the window's
    /// current committed size.
    fn centered_float_geometry(&self, window: &Window) -> Rectangle<i32, Logical> {
        let output_geo = self
            .visible_workspace_of_window(window)
            .and_then(|(_, monitor)| self.monitors.get(monitor))
            .or_else(|| self.monitors.get(self.active_monitor))
            .and_then(|monitor| self.space.output_geometry(&monitor.output))
            .unwrap_or_else(|| Rectangle::new(Point::from((0, 0)), Size::from((800, 600))));

        let size = window.geometry().size;
        let size = Size::from((
            size.w.clamp(1, output_geo.size.w),
            size.h.clamp(1, output_geo.size.h),
        ));
        Rectangle::new(
            Point::from((
                output_geo.loc.x + (output_geo.size.w - size.w) / 2,
                output_geo.loc.y + (output_geo.size.h - size.h) / 2,
            )),
            size,
        )
    }

    /// Swaps a freshly inserted master `window` one slot down its workspace
    /// stack (`doF W.swapDown` after the default master insertion).
    fn swap_new_master_down(&mut self, window: &Window) {
        for workspace in &mut self.workspaces {
            if let Some(index) = workspace.windows.iter().position(|w| w == window)
                && index + 1 < workspace.windows.len()
            {
                workspace.windows.swap(index, index + 1);
            }
        }
    }
}

/// Reads the committed title and app id of an xdg toplevel surface.
fn surface_meta(surface: &WlSurface) -> (Option<String>, Option<String>) {
    with_states(surface, |states| {
        states
            .data_map
            .get::<XdgToplevelSurfaceData>()
            .and_then(|data| data.lock().ok())
            .map_or((None, None), |data| {
                (data.title.clone(), data.app_id.clone())
            })
    })
}
