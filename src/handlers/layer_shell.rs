//! `wlr-layer-shell` support.
//!
//! Layer-shell clients (application launchers like tofi, status bars,
//! wallpaper and notification daemons) anchor themselves to an output rather
//! than being tiled. We track them in the per-output [`LayerMap`], which
//! `Space::render_output` already renders at the correct z-order, and grab the
//! keyboard for interactive ones so a launcher can be typed into.
//!
//! [`LayerMap`]: smithay::desktop::LayerMap

use smithay::{
    delegate_layer_shell,
    desktop::{
        LayerSurface as DesktopLayerSurface,
        WindowSurfaceType,
        layer_map_for_output,
    },
    output::Output,
    reexports::wayland_server::protocol::{
        wl_output::WlOutput,
        wl_surface::WlSurface,
    },
    utils::SERIAL_COUNTER,
    wayland::{
        compositor::with_states,
        shell::wlr_layer::{
            Layer,
            LayerSurface,
            LayerSurfaceData,
            WlrLayerShellHandler,
            WlrLayerShellState,
        },
    },
};

use crate::Oxmonad;

impl WlrLayerShellHandler for Oxmonad {
    fn shell_state(&mut self) -> &mut WlrLayerShellState {
        &mut self.layer_shell_state
    }

    fn new_layer_surface(
        &mut self,
        surface: LayerSurface,
        output: Option<WlOutput>,
        _layer: Layer,
        namespace: String,
    ) {
        // Honor the requested output, otherwise fall back to the only output.
        let Some(output) = output
            .as_ref()
            .and_then(Output::from_resource)
            .or_else(|| self.space.outputs().next().cloned())
        else {
            return;
        };

        let mut map = layer_map_for_output(&output);
        // `map_layer` arranges the layer (anchors, exclusive zones).
        let _ = map.map_layer(&DesktopLayerSurface::new(surface, namespace));
        drop(map);

        // Exclusive zones may have shrunk the tiling area.
        self.relayout();
    }

    fn layer_destroyed(&mut self, surface: LayerSurface) {
        let mut output = None;
        for candidate in self.space.outputs() {
            if layer_map_for_output(candidate)
                .layer_for_surface(surface.wl_surface(), WindowSurfaceType::ALL)
                .is_some()
            {
                output = Some(candidate.clone());
                break;
            }
        }

        if let Some(output) = output {
            let mut map = layer_map_for_output(&output);
            if let Some(layer) = map
                .layer_for_surface(surface.wl_surface(), WindowSurfaceType::ALL)
                .cloned()
            {
                map.unmap_layer(&layer);
            }
        }

        self.relayout();
        // Return keyboard focus to the active monitor's master window after the
        // layer (e.g. a launcher) goes away.
        if let Some(master) = self
            .monitors
            .get(self.active_monitor)
            .and_then(|monitor| monitor.windows.first())
            .cloned()
        {
            self.focus_window(&master);
        }
    }
}

delegate_layer_shell!(Oxmonad);

/// Sends the initial configure to a freshly-committed layer surface and grabs
/// the keyboard for interactive layers. Called from the compositor commit hook.
pub fn handle_commit(state: &mut Oxmonad, surface: &WlSurface) {
    let Some(output) = state
        .space
        .outputs()
        .find(|output| {
            layer_map_for_output(output)
                .layer_for_surface(surface, WindowSurfaceType::ALL)
                .is_some()
        })
        .cloned()
    else {
        return;
    };

    let mut map = layer_map_for_output(&output);
    map.arrange();
    let layer = map
        .layer_for_surface(surface, WindowSurfaceType::ALL)
        .cloned();
    drop(map);

    let Some(layer) = layer else {
        return;
    };

    let initial_configure_sent = with_states(surface, |states| {
        states
            .data_map
            .get::<LayerSurfaceData>()
            .expect("layer surface always has LayerSurfaceData")
            .lock()
            .expect("layer surface state mutex poisoned")
            .initial_configure_sent
    });

    if !initial_configure_sent {
        layer.layer_surface().send_configure();

        // Hand the keyboard to interactive layers (e.g. a launcher) so they can
        // be typed into.
        if layer.can_receive_keyboard_focus() {
            let serial = SERIAL_COUNTER.next_serial();
            if let Some(keyboard) = state.seat.get_keyboard() {
                keyboard.set_focus(state, Some(surface.clone()), serial);
            }
        }
    }
}
