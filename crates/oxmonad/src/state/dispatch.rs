//! Event dispatch: records river's events into the [`State`] mirror and
//! input queue; the sequences themselves run in the parent module.

use std::{
    process::ExitCode,
    sync::atomic::Ordering,
};

use oxmonad_core::{
    Rect,
    ScreenId,
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
    Input,
    KeyBindingData,
    Output,
    PointerBindingData,
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
                let screen = *id.data::<ScreenId>().unwrap_or(&ScreenId(0));
                let layer = state
                    .globals
                    .layer_shell
                    .as_ref()
                    .map(|ls| ls.get_output(&id, qh, screen));
                state.outputs.insert(
                    screen,
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
        river_window_manager_v1::EVT_OUTPUT_OPCODE => (RiverOutputV1, ScreenId(next_id())),
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
                let preferred =
                    if max_width > 0 && max_width == min_width && max_height == min_height {
                        Some(Size::new(max_width, max_height))
                    } else {
                        (min_width > 0 && min_height > 0).then(|| Size::new(min_width, min_height))
                    };
                window.info.preferred_size = preferred;
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

impl Dispatch<RiverOutputV1, ScreenId> for State {
    fn event(
        state: &mut Self,
        _: &RiverOutputV1,
        event: river_output_v1::Event,
        id: &ScreenId,
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

impl Dispatch<RiverLayerShellOutputV1, ScreenId> for State {
    fn event(
        state: &mut Self,
        _: &RiverLayerShellOutputV1,
        event: river_layer_shell_output_v1::Event,
        id: &ScreenId,
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
            Event::ShellSurfaceInteraction { shell_surface } => {
                if let Some(&node) = shell_surface.data::<Node>() {
                    state.inputs.push_back(Input::ShellClick(id, node));
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
            _ => {}
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
        let prompt_open = state.wm.is_prompt_open();
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
