//! Event dispatch: records river's events into the [`State`] mirror and
//! input queue; the sequences themselves run in the parent module.

use std::{
    process::ExitCode,
    sync::atomic::Ordering,
};

use oxmonad_core::{
    OutputId,
    Rect,
    Size,
    WindowId,
    WindowInfo,
};
use river_protocols::{
    layer_shell::{
        river_layer_shell_output_v1::{
            self,
            RiverLayerShellOutputV1,
        },
        river_layer_shell_seat_v1::{
            self,
            RiverLayerShellSeatV1,
        },
        river_layer_shell_v1::RiverLayerShellV1,
    },
    window_management::{
        river_node_v1::RiverNodeV1,
        river_output_v1::{
            self,
            RiverOutputV1,
        },
        river_pointer_binding_v1::{
            self,
            RiverPointerBindingV1,
        },
        river_seat_v1::{
            self,
            RiverSeatV1,
        },
        river_shell_surface_v1::RiverShellSurfaceV1,
        river_window_manager_v1::{
            self,
            RiverWindowManagerV1,
        },
        river_window_v1::{
            self,
            RiverWindowV1,
        },
    },
    xkb_bindings::{
        river_xkb_binding_v1::{
            self,
            RiverXkbBindingV1,
        },
        river_xkb_bindings_v1::RiverXkbBindingsV1,
    },
};
use wayland_client::{
    Connection,
    Dispatch,
    Proxy,
    QueueHandle,
    WEnum,
    delegate_noop,
    event_created_child,
    globals::GlobalListContents,
    protocol::{
        wl_buffer::{
            self,
            WlBuffer,
        },
        wl_compositor::WlCompositor,
        wl_keyboard::{
            self,
            KeyState,
            KeymapFormat,
            WlKeyboard,
        },
        wl_output::{
            self,
            WlOutput,
        },
        wl_pointer::{
            self,
            ButtonState,
            WlPointer,
        },
        wl_registry::{
            self,
            WlRegistry,
        },
        wl_seat::{
            self,
            WlSeat,
        },
        wl_shm::WlShm,
        wl_shm_pool::WlShmPool,
        wl_surface::WlSurface,
    },
};

use super::{
    Click,
    Input,
    KeyBindingData,
    Output,
    PointerBindingData,
    Scroll,
    Seat,
    SeatId,
    State,
    Window,
    next_id,
};
use crate::{
    keyboard::Keyboard,
    plan::Node,
    shm::BusyFlag,
};

impl Dispatch<RiverWindowManagerV1, ()> for State {
    fn event(
        state: &mut Self,
        wm: &RiverWindowManagerV1,
        event: river_window_manager_v1::Event,
        (): &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        use river_window_manager_v1::Event;
        match event {
            Event::Unavailable => {
                tracing::error!("river refused window management: another window manager runs");
                state.exit = Some(ExitCode::FAILURE);
            }
            Event::Finished if state.is_restarting() => state.finish_restart(),
            Event::Finished => {
                tracing::info!("river finished window management");
                wm.destroy();
                state.exit = Some(ExitCode::SUCCESS);
            }
            Event::ManageStart => state.manage_sequence(),
            Event::RenderStart => state.render_sequence(),
            Event::SessionLocked => state.locked = true,
            Event::SessionUnlocked => state.locked = false,
            Event::Window { id } => {
                let window_id = *id.data::<WindowId>().unwrap_or(&WindowId(0));
                let node = id.get_node(qh, ());
                state.windows.insert(
                    window_id,
                    Window {
                        proxy: id,
                        node,
                        identifier: None,
                        info: WindowInfo::default(),
                        dimensions: None,
                        managed: false,
                        info_changed: false,
                        closed: false,
                    },
                );
            }
            Event::Output { id } => {
                let output = *id.data::<OutputId>().unwrap_or(&OutputId(0));
                let layer = state
                    .globals
                    .layer_shell
                    .as_ref()
                    .map(|ls| ls.get_output(&id, qh, output));
                state.outputs.insert(
                    output,
                    Output {
                        proxy: id,
                        layer,
                        global: None,
                        position: None,
                        size: None,
                        usable: None,
                        added: false,
                        changed: false,
                        removed: false,
                    },
                );
            }
            Event::Seat { id } => {
                let seat = *id.data::<SeatId>().unwrap_or(&SeatId(0));
                let layer = state
                    .globals
                    .layer_shell
                    .as_ref()
                    .map(|ls| ls.get_seat(&id, qh, seat));
                state.seats.insert(
                    seat,
                    Seat {
                        proxy: id,
                        layer,
                        wl: None,
                        keyboard: None,
                        wl_pointer: None,
                        pointer_surface: None,
                        pointer_local: None,
                        hovered_layout: None,
                        hovered_menu: None,
                        scroll: Scroll::default(),
                        pointer: None,
                        pointer_before: None,
                        hovered: None,
                        removed: false,
                    },
                );
            }
            _ => {}
        }
    }

    event_created_child!(State, RiverWindowManagerV1, [
        river_window_manager_v1::EVT_WINDOW_OPCODE => (RiverWindowV1, WindowId(next_id())),
        river_window_manager_v1::EVT_OUTPUT_OPCODE => (RiverOutputV1, OutputId(next_id())),
        river_window_manager_v1::EVT_SEAT_OPCODE => (RiverSeatV1, SeatId(next_id())),
    ]);
}

impl Dispatch<RiverWindowV1, WindowId> for State {
    fn event(
        state: &mut Self,
        _: &RiverWindowV1,
        event: river_window_v1::Event,
        id: &WindowId,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use river_window_v1::Event;
        let id = *id;
        let Some(window) = state.windows.get_mut(&id) else {
            return;
        };
        match event {
            Event::Closed => window.closed = true,
            Event::Dimensions { width, height } => {
                window.dimensions = Some(Size::new(width, height));
            }
            Event::DimensionsHint {
                min_width,
                min_height,
                max_width,
                max_height,
            } => {
                window.info.preferred_size =
                    preferred_size(min_width, min_height, max_width, max_height);
                window.info_changed = true;
            }
            Event::AppId { app_id } => {
                window.info.app_id = app_id.unwrap_or_default();
                window.info_changed = true;
            }
            Event::Title { title } => {
                window.info.title = title.unwrap_or_default();
                window.info_changed = true;
            }
            Event::Identifier { identifier } => window.identifier = Some(identifier),
            Event::Parent { parent } => {
                window.info.has_parent = parent.is_some();
                window.info_changed = true;
            }
            Event::PointerMoveRequested { seat } => {
                if let Some(&seat) = seat.data::<SeatId>() {
                    state.inputs.push_back(Input::MoveRequested(seat, id));
                }
            }
            Event::PointerResizeRequested { seat, .. } => {
                if let Some(&seat) = seat.data::<SeatId>() {
                    state.inputs.push_back(Input::ResizeRequested(seat, id));
                }
            }
            Event::FullscreenRequested { .. } => {
                state.inputs.push_back(Input::Fullscreen(id, true));
            }
            Event::ExitFullscreenRequested => {
                state.inputs.push_back(Input::Fullscreen(id, false));
            }
            Event::DecorationHint { .. }
            | Event::ShowWindowMenuRequested { .. }
            | Event::MaximizeRequested
            | Event::UnmaximizeRequested
            | Event::MinimizeRequested
            | Event::UnreliablePid { .. }
            | Event::PresentationHint { .. }
            | Event::CaptureSessions { .. }
            | _ => {}
        }
    }
}

impl Dispatch<RiverOutputV1, OutputId> for State {
    fn event(
        state: &mut Self,
        _: &RiverOutputV1,
        event: river_output_v1::Event,
        id: &OutputId,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use river_output_v1::Event;
        let Some(output) = state.outputs.get_mut(id) else {
            return;
        };
        match event {
            Event::Removed => output.removed = true,
            Event::WlOutput { name } => {
                output.global = Some(name);
                output.changed = true;
            }
            Event::Position { x, y } => {
                output.position = Some((x, y));
                output.changed = true;
            }
            Event::Dimensions { width, height } => {
                output.size = Some((width, height));
                output.changed = true;
            }
            Event::CaptureSessions { .. } | _ => {}
        }
    }
}

impl Dispatch<RiverLayerShellOutputV1, OutputId> for State {
    fn event(
        state: &mut Self,
        _: &RiverLayerShellOutputV1,
        event: river_layer_shell_output_v1::Event,
        id: &OutputId,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use river_layer_shell_output_v1::Event;
        if let (
            Event::NonExclusiveArea {
                x,
                y,
                width,
                height,
            },
            Some(output),
        ) = (event, state.outputs.get_mut(id))
        {
            output.usable = Some(Rect::new(x, y, width, height));
            output.changed = true;
        }
    }
}

impl Dispatch<RiverSeatV1, SeatId> for State {
    fn event(
        state: &mut Self,
        _: &RiverSeatV1,
        event: river_seat_v1::Event,
        id: &SeatId,
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        use river_seat_v1::Event;
        let id = *id;
        let window_id = |w: &RiverWindowV1| w.data::<WindowId>().copied();
        match event {
            Event::Removed => {
                if let Some(seat) = state.seats.get_mut(&id) {
                    seat.removed = true;
                }
            }
            Event::WlSeat { name } => {
                let version = state.globals.list.contents().with_list(|list| {
                    list.iter()
                        .find(|g| g.name == name && g.interface == WlSeat::interface().name)
                        .map(|g| g.version)
                });
                let Some(version) = version else {
                    tracing::warn!(name, "river named an unknown wl_seat");
                    return;
                };
                let wl_seat: WlSeat =
                    state
                        .globals
                        .list
                        .registry()
                        .bind(name, version.min(7), qh, id);
                if let Some(seat) = state.seats.get_mut(&id) {
                    seat.wl = Some(wl_seat);
                }
            }
            Event::PointerEnter { window } => {
                let Some(window) = window_id(&window) else {
                    return;
                };
                if let Some(seat) = state.seats.get_mut(&id) {
                    seat.hovered = Some(window);
                }
                state.inputs.push_back(Input::Hover(id, window));
            }
            Event::PointerLeave => {
                if let Some(seat) = state.seats.get_mut(&id) {
                    seat.hovered = None;
                }
            }
            Event::WindowInteraction { window } => {
                if let Some(window) = window_id(&window) {
                    state.inputs.push_back(Input::Interaction(window));
                }
            }
            Event::OpDelta { dx, dy } => {
                if let Some(op) = state.op.as_mut().filter(|op| op.seat == id) {
                    op.delta = Some((dx, dy));
                }
            }
            Event::OpRelease => {
                if let Some(op) = state.op.as_mut().filter(|op| op.seat == id) {
                    op.released = true;
                }
            }
            Event::PointerPosition { x, y } => {
                if let Some(seat) = state.seats.get_mut(&id) {
                    seat.pointer = Some((x, y));
                }
            }
            // `shell_surface_interaction` is ignored: shell surface clicks
            // come from our own `wl_pointer`, which knows the button and
            // the exact position (see its dispatch).
            Event::ShellSurfaceInteraction { .. } | _ => {}
        }
    }
}

impl Dispatch<RiverLayerShellSeatV1, SeatId> for State {
    fn event(
        state: &mut Self,
        _: &RiverLayerShellSeatV1,
        event: river_layer_shell_seat_v1::Event,
        _: &SeatId,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // A layer surface (a launcher, a lock screen…) took or released
        // keyboard focus behind our back: resend ours when it is gone.
        if matches!(event, river_layer_shell_seat_v1::Event::FocusNone) {
            state.applied.invalidate_focus();
        }
    }
}

impl Dispatch<RiverXkbBindingV1, KeyBindingData> for State {
    fn event(
        state: &mut Self,
        _: &RiverXkbBindingV1,
        event: river_xkb_binding_v1::Event,
        data: &KeyBindingData,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if matches!(event, river_xkb_binding_v1::Event::Pressed) {
            state.inputs.push_back(Input::Key(data.clone()));
        }
    }
}

impl Dispatch<RiverPointerBindingV1, PointerBindingData> for State {
    fn event(
        state: &mut Self,
        _: &RiverPointerBindingV1,
        event: river_pointer_binding_v1::Event,
        data: &PointerBindingData,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if matches!(event, river_pointer_binding_v1::Event::Pressed) {
            state.inputs.push_back(Input::Pointer(data.clone()));
        }
    }
}

impl Dispatch<WlSeat, SeatId> for State {
    fn event(
        state: &mut Self,
        wl_seat: &WlSeat,
        event: wl_seat::Event,
        id: &SeatId,
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Capabilities {
            capabilities: WEnum::Value(caps),
        } = event
            && let Some(seat) = state.seats.get_mut(id)
        {
            let has_keyboard = caps.contains(wl_seat::Capability::Keyboard);
            match (&seat.keyboard, has_keyboard) {
                (None, true) => {
                    seat.keyboard = Some(Keyboard::new(wl_seat.get_keyboard(qh, *id)));
                }
                (Some(_), false) => {
                    if let Some(keyboard) = seat.keyboard.take() {
                        keyboard.release();
                    }
                }
                _ => {}
            }
            let has_pointer = caps.contains(wl_seat::Capability::Pointer);
            match (&seat.wl_pointer, has_pointer) {
                (None, true) => seat.wl_pointer = Some(wl_seat.get_pointer(qh, *id)),
                (Some(_), false) => {
                    if let Some(pointer) = seat.wl_pointer.take()
                        && pointer.version() >= 3
                    {
                        pointer.release();
                    }
                    seat.pointer_surface = None;
                }
                _ => {}
            }
        }
    }
}

impl Dispatch<WlPointer, SeatId> for State {
    fn event(
        state: &mut Self,
        pointer: &WlPointer,
        event: wl_pointer::Event,
        id: &SeatId,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(seat) = state.seats.get_mut(id) else {
            return;
        };
        match event {
            wl_pointer::Event::Enter {
                surface,
                surface_x,
                surface_y,
                ..
            } => {
                seat.pointer_surface = Some(surface);
                seat.pointer_local = Some((surface_x, surface_y));
                state.pointer_hover(*id);
            }
            wl_pointer::Event::Motion {
                surface_x,
                surface_y,
                ..
            } => {
                seat.pointer_local = Some((surface_x, surface_y));
                state.pointer_hover(*id);
            }
            wl_pointer::Event::Leave { .. } => {
                seat.pointer_surface = None;
                seat.pointer_local = None;
                seat.hovered_layout = None;
                seat.hovered_menu = None;
                seat.scroll = Scroll::default();
            }
            wl_pointer::Event::Button {
                button,
                state: WEnum::Value(ButtonState::Pressed),
                ..
            } => {
                let (Some(surface), Some((x, y))) = (&seat.pointer_surface, seat.pointer_local)
                else {
                    return;
                };
                let Some((node, x, y)) = state.shells.to_global(surface, x, y, &state.styles)
                else {
                    return;
                };
                state.inputs.push_back(Input::ShellButton {
                    seat: *id,
                    node,
                    x,
                    y,
                    button,
                });
                state.dirty();
            }
            wl_pointer::Event::AxisDiscrete { axis, discrete } => match axis {
                WEnum::Value(wl_pointer::Axis::VerticalScroll) => {
                    seat.scroll.discrete.0 += discrete;
                }
                WEnum::Value(wl_pointer::Axis::HorizontalScroll) => {
                    seat.scroll.discrete.1 += discrete;
                }
                WEnum::Value(_) | WEnum::Unknown(_) => {}
            },
            wl_pointer::Event::Axis { axis, value, .. } => {
                match axis {
                    WEnum::Value(wl_pointer::Axis::VerticalScroll) => {
                        seat.scroll.continuous.0 += value;
                    }
                    WEnum::Value(wl_pointer::Axis::HorizontalScroll) => {
                        seat.scroll.continuous.1 += value;
                    }
                    WEnum::Value(_) | WEnum::Unknown(_) => {}
                }
                // Before version 5 there are no frames.
                if pointer.version() < wl_pointer::EVT_FRAME_SINCE {
                    state.pointer_scroll(*id);
                }
            }
            wl_pointer::Event::Frame => state.pointer_scroll(*id),
            wl_pointer::Event::Button { .. }
            | wl_pointer::Event::AxisSource { .. }
            | wl_pointer::Event::AxisStop { .. }
            | wl_pointer::Event::AxisValue120 { .. }
            | wl_pointer::Event::AxisRelativeDirection { .. }
            | _ => {}
        }
    }
}

impl State {
    /// The pointer of seat `id` moved over one of our surfaces: hovering
    /// selects layout picker entries (like `j`/`k`) and highlights tray
    /// menu rows.
    fn pointer_hover(&mut self, id: SeatId) {
        let Some(seat) = self.seats.get(&id) else {
            return;
        };
        let (Some(surface), Some((x, y))) = (&seat.pointer_surface, seat.pointer_local) else {
            return;
        };
        let click = self.shells.at_local(surface, x, y, &self.styles);
        let Some(seat) = self.seats.get_mut(&id) else {
            return;
        };
        match click {
            Some(Click::Layout(index)) if seat.hovered_layout != Some(index) => {
                seat.hovered_layout = Some(index);
                self.inputs.push_back(Input::PickerHover(index));
                self.dirty();
            }
            Some(Click::MenuEntry(level, row)) if seat.hovered_menu != Some((level, row)) => {
                seat.hovered_menu = Some((level, row));
                self.inputs.push_back(Input::MenuHover(level, row));
                self.dirty();
            }
            _ => {}
        }
    }

    /// A `wl_pointer` frame ended: scrolling over a tray icon goes to its
    /// item right away (it changes nothing oxmonad manages).
    fn pointer_scroll(&mut self, id: SeatId) {
        let Some(seat) = self.seats.get_mut(&id) else {
            return;
        };
        let (vertical, horizontal) = seat.scroll.take_steps();
        if (vertical, horizontal) == (0, 0) {
            return;
        }
        let (Some(surface), Some((x, y))) = (&seat.pointer_surface, seat.pointer_local) else {
            return;
        };
        let (Some(Click::TrayItem(item)), Some(handle)) = (
            self.shells.at_local(surface, x, y, &self.styles),
            &self.tray.thread,
        ) else {
            return;
        };
        if vertical != 0 {
            handle.tray.scroll(&item, vertical, false);
        }
        if horizontal != 0 {
            handle.tray.scroll(&item, horizontal, true);
        }
    }
}

impl Dispatch<WlKeyboard, SeatId> for State {
    fn event(
        state: &mut Self,
        _: &WlKeyboard,
        event: wl_keyboard::Event,
        id: &SeatId,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let prompt_open = state.wm.grabs_keyboard();
        let menu_open = state.tray.menu.is_some();
        let Some(keyboard) = state.seats.get_mut(id).and_then(|s| s.keyboard.as_mut()) else {
            return;
        };
        match event {
            wl_keyboard::Event::Keymap { format, fd, size } => {
                if format == WEnum::Value(KeymapFormat::XkbV1) {
                    keyboard.set_keymap(&fd, size);
                }
            }
            wl_keyboard::Event::Modifiers {
                mods_depressed,
                mods_latched,
                mods_locked,
                group,
                ..
            } => keyboard.set_modifiers(mods_depressed, mods_latched, mods_locked, group),
            wl_keyboard::Event::RepeatInfo { rate, delay } => keyboard.set_repeat(rate, delay),
            wl_keyboard::Event::Key {
                key,
                state: WEnum::Value(KeyState::Pressed),
                ..
            } if prompt_open => {
                let repeat = keyboard.repeat(key);
                if let Some(prompt_key) = keyboard.prompt_key(key) {
                    state.inputs.push_back(Input::PromptKey(prompt_key));
                    state.dirty();
                    if let Some(timing) = repeat {
                        state.start_repeat(*id, key, prompt_key, timing);
                    }
                }
            }
            wl_keyboard::Event::Key {
                key,
                state: WEnum::Value(KeyState::Pressed),
                ..
            } if menu_open => {
                if let Some(menu_key) = keyboard.prompt_key(key) {
                    state.inputs.push_back(Input::MenuKey(menu_key));
                    state.dirty();
                }
            }
            wl_keyboard::Event::Key {
                key,
                state: WEnum::Value(KeyState::Released),
                ..
            } => state.stop_repeat(Some(key)),
            wl_keyboard::Event::Leave { .. } => state.stop_repeat(None),
            wl_keyboard::Event::Enter { .. } | wl_keyboard::Event::Key { .. } | _ => {}
        }
    }
}

impl Dispatch<WlOutput, u32> for State {
    fn event(
        state: &mut Self,
        _: &WlOutput,
        event: wl_output::Event,
        name: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(info) = state.wl_outputs.get_mut(name) else {
            return;
        };
        match event {
            wl_output::Event::Scale { factor } => info.scale = factor,
            wl_output::Event::Name { name: output_name } => info.name = output_name,
            wl_output::Event::Done => {
                // Name or scale may have changed: refresh the screen.
                for output in state.outputs.values_mut() {
                    if output.global == Some(*name) {
                        output.changed = true;
                    }
                }
                state.dirty();
            }
            wl_output::Event::Geometry { .. }
            | wl_output::Event::Mode { .. }
            | wl_output::Event::Description { .. }
            | _ => {}
        }
    }
}

impl Dispatch<WlRegistry, GlobalListContents> for State {
    fn event(
        state: &mut Self,
        _: &WlRegistry,
        event: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_registry::Event::Global {
                name,
                interface,
                version,
            } if interface == WlOutput::interface().name => state.bind_wl_output(name, version),
            wl_registry::Event::GlobalRemove { name } => {
                if let Some(info) = state.wl_outputs.remove(&name) {
                    info.output.release();
                }
            }
            wl_registry::Event::Global { .. } | _ => {}
        }
    }
}

impl Dispatch<WlBuffer, BusyFlag> for State {
    fn event(
        _: &mut Self,
        _: &WlBuffer,
        event: wl_buffer::Event,
        busy: &BusyFlag,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if matches!(event, wl_buffer::Event::Release) {
            busy.store(false, Ordering::Release);
        }
    }
}

delegate_noop!(State: ignore WlCompositor);
delegate_noop!(State: ignore WlSurface);
delegate_noop!(State: ignore WlShm);
delegate_noop!(State: WlShmPool);
delegate_noop!(State: RiverNodeV1);
delegate_noop!(State: RiverXkbBindingsV1);
delegate_noop!(State: RiverLayerShellV1);

impl Dispatch<RiverShellSurfaceV1, Node> for State {
    fn event(
        _: &mut Self,
        _: &RiverShellSurfaceV1,
        _: <RiverShellSurfaceV1 as Proxy>::Event,
        _: &Node,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

/// The size a floating window should get from its dimension hints.
///
/// Only a fixed-size window (min == max) states a size it wants; a minimum
/// alone is a lower bound, not a preference (alacritty's is a few pixels), so
/// such windows get core's default float size instead.
fn preferred_size(
    min_width: i32,
    min_height: i32,
    max_width: i32,
    max_height: i32,
) -> Option<Size> {
    (min_width > 0 && min_height > 0 && min_width == max_width && min_height == max_height)
        .then(|| Size::new(min_width, min_height))
}

#[cfg(test)]
mod preferred_size_tests {
    use super::*;

    #[test]
    fn fixed_size_windows_keep_their_size() {
        assert_eq!(
            preferred_size(400, 300, 400, 300),
            Some(Size::new(400, 300))
        );
    }

    #[test]
    fn a_minimum_alone_is_not_a_preference() {
        assert_eq!(preferred_size(2, 1, 0, 0), None);
        assert_eq!(preferred_size(200, 100, 800, 600), None);
    }

    #[test]
    fn missing_hints_give_no_preference() {
        assert_eq!(preferred_size(0, 0, 0, 0), None);
    }
}
