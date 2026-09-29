//! The river client: mirrors river's objects into core IDs, records events
//! and runs the manage and render sequences.
//!
//! Core state only changes in manage sequences (see
//! `docs/architecture.adoc`): events are recorded into [`Pending`] and
//! [`Input`]s and applied on `manage_start`.

use std::{
    collections::{
        BTreeMap,
        VecDeque,
    },
    path::PathBuf,
    process::ExitCode,
    sync::atomic::{
        AtomicU64,
        Ordering,
    },
};

use oxmonad_core::{
    Action,
    Arrangement,
    Border,
    Effect,
    Focus,
    Rect,
    ScreenId,
    Size,
    WindowId,
    WindowInfo,
    Wm,
    config::Config,
    prompt::PromptKey,
};
use river_protocols::{
    layer_shell::{
        river_layer_shell_output_v1::RiverLayerShellOutputV1,
        river_layer_shell_seat_v1::RiverLayerShellSeatV1,
        river_layer_shell_v1::RiverLayerShellV1,
    },
    window_management::{
        river_node_v1::RiverNodeV1,
        river_output_v1::RiverOutputV1,
        river_pointer_binding_v1::RiverPointerBindingV1,
        river_seat_v1::{
            self,
            RiverSeatV1,
        },
        river_window_manager_v1::{
            self,
            RiverWindowManagerV1,
        },
        river_window_v1::{
            Capabilities,
            Edges,
            RiverWindowV1,
        },
    },
    xkb_bindings::{
        river_xkb_binding_v1::RiverXkbBindingV1,
        river_xkb_bindings_v1::RiverXkbBindingsV1,
    },
};
use wayland_client::{
    Connection,
    Proxy,
    QueueHandle,
    globals::GlobalList,
    protocol::{
        wl_compositor::WlCompositor,
        wl_output::WlOutput,
        wl_seat::WlSeat,
        wl_shm::WlShm,
    },
};

mod dispatch;

use crate::{
    config_file,
    control::{
        self,
        Command,
        Reply,
    },
    keyboard::Keyboard,
    keys,
    plan::{
        self,
        Applied,
        Node,
        Requests,
    },
    shell::{
        self,
        Click,
        Content,
        FontCache,
        ShellGlobals,
        Shells,
        Status,
        Styles,
        Wanted,
    },
    spawn::Children,
};

/// Identifies a river seat.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SeatId(u64);

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

fn next_id() -> u64 {
    NEXT_ID.fetch_add(1, Ordering::Relaxed)
}

/// The globals oxmonad needs.
#[derive(Debug)]
pub struct Globals {
    /// The registry and its advertised globals.
    pub list: GlobalList,
    /// The window manager global.
    pub wm: RiverWindowManagerV1,
    /// Key bindings.
    pub xkb: Option<RiverXkbBindingsV1>,
    /// Layer-shell support.
    pub layer_shell: Option<RiverLayerShellV1>,
    /// For shell surfaces.
    pub compositor: WlCompositor,
    /// For shell surface buffers.
    pub shm: WlShm,
}

#[derive(Debug)]
struct Window {
    proxy: RiverWindowV1,
    node: RiverNodeV1,
    info: WindowInfo,
    /// Dimensions river reported.
    dimensions: Option<Size>,
    managed: bool,
    info_changed: bool,
    closed: bool,
}

#[derive(Debug)]
struct Output {
    proxy: RiverOutputV1,
    layer: Option<RiverLayerShellOutputV1>,
    /// The `wl_output` global name.
    global: Option<u32>,
    position: Option<(i32, i32)>,
    size: Option<(i32, i32)>,
    usable: Option<Rect>,
    /// Told core about it.
    added: bool,
    changed: bool,
    removed: bool,
}

impl Output {
    fn rect(&self) -> Option<Rect> {
        let ((x, y), (w, h)) = (self.position?, self.size?);
        Some(Rect::new(x, y, w, h))
    }
}

/// What oxmonad knows about a `wl_output` global.
#[derive(Debug)]
struct WlOutputInfo {
    output: WlOutput,
    name: String,
    scale: i32,
}

#[derive(Debug)]
struct Seat {
    proxy: RiverSeatV1,
    layer: Option<RiverLayerShellSeatV1>,
    wl: Option<WlSeat>,
    keyboard: Option<Keyboard>,
    pointer: Option<(i32, i32)>,
    /// The pointer position in the previous manage sequence.
    pointer_before: Option<(i32, i32)>,
    hovered: Option<WindowId>,
    removed: bool,
}

/// Data of a key binding object.
#[derive(Debug, Clone)]
pub struct KeyBindingData {
    action: Action,
    /// Whether the combination has modifiers (unmodified ones, i.e. media
    /// keys, keep working while the session is locked).
    modified: bool,
}

/// Data of a pointer binding object.
#[derive(Debug, Clone)]
pub struct PointerBindingData {
    action: Action,
    seat: SeatId,
}

#[derive(Debug)]
struct SeatBindings {
    keys: Vec<RiverXkbBindingV1>,
    pointer: Vec<RiverPointerBindingV1>,
    enabled: bool,
}

impl SeatBindings {
    fn destroy(self) {
        self.keys.iter().for_each(RiverXkbBindingV1::destroy);
        self.pointer.iter().for_each(RiverPointerBindingV1::destroy);
    }
}

/// User intent recorded between manage sequences, applied in order.
#[derive(Debug)]
enum Input {
    Key(KeyBindingData),
    Pointer(PointerBindingData),
    PromptKey(PromptKey),
    Interaction(WindowId),
    Hover(SeatId, WindowId),
    ShellClick(SeatId, Node),
    MoveRequested(SeatId, WindowId),
    ResizeRequested(SeatId, WindowId),
    Fullscreen(WindowId, bool),
    Control(Command, Reply),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OpKind {
    Move,
    Resize,
}

#[derive(Debug)]
struct PointerOp {
    seat: SeatId,
    window: WindowId,
    kind: OpKind,
    start: Rect,
    delta: Option<(i32, i32)>,
    released: bool,
}

/// Whether oxmonad is shutting down.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lifecycle {
    Running,
    /// Sent `stop`, waiting for `finished`.
    Stopping,
    /// Asked river to end the session.
    Quitting,
}

/// Everything the event loop works on.
#[derive(Debug)]
pub struct State {
    conn: Connection,
    qh: QueueHandle<Self>,
    globals: Globals,
    wm: Wm,
    config_path: Option<PathBuf>,
    fonts: FontCache,
    styles: Styles,
    status: Status,

    windows: BTreeMap<WindowId, Window>,
    outputs: BTreeMap<ScreenId, Output>,
    wl_outputs: BTreeMap<u32, WlOutputInfo>,
    seats: BTreeMap<SeatId, Seat>,
    bindings: BTreeMap<SeatId, SeatBindings>,

    inputs: VecDeque<Input>,
    op: Option<PointerOp>,
    applied: Applied,
    arrangement: Option<Arrangement>,
    shells: Shells,
    /// Control commands applied in the running manage sequence, answered
    /// after the next render sequence.
    answers: Vec<(Command, Reply)>,
    children: Children,

    locked: bool,
    reload: bool,
    lifecycle: Lifecycle,
    exit: Option<ExitCode>,
}

impl State {
    /// A fresh state around the bound globals.
    pub fn new(
        conn: Connection,
        qh: QueueHandle<Self>,
        globals: Globals,
        config: Config,
        config_path: Option<PathBuf>,
    ) -> Self {
        let mut fonts = FontCache::default();
        let styles = Styles::new(&config, &mut fonts);
        let mut state = Self {
            conn,
            qh,
            globals,
            wm: Wm::new(config),
            config_path,
            fonts,
            styles,
            status: Status::default(),
            windows: BTreeMap::new(),
            outputs: BTreeMap::new(),
            wl_outputs: BTreeMap::new(),
            seats: BTreeMap::new(),
            bindings: BTreeMap::new(),
            inputs: VecDeque::new(),
            op: None,
            applied: Applied::default(),
            arrangement: None,
            shells: Shells::default(),
            answers: Vec::new(),
            children: Children::default(),
            locked: false,
            reload: false,
            lifecycle: Lifecycle::Running,
            exit: None,
        };
        let outputs: Vec<(u32, u32)> = state.globals.list.contents().with_list(|list| {
            list.iter()
                .filter(|g| g.interface == WlOutput::interface().name)
                .map(|g| (g.name, g.version))
                .collect()
        });
        for (name, version) in outputs {
            state.bind_wl_output(name, version);
        }
        state
    }

    /// Flushes requests; called whenever the event loop goes idle.
    pub fn flush(&self) {
        if let Err(err) = self.conn.flush() {
            tracing::debug!(%err, "flush failed");
        }
    }

    /// Why the event loop should stop, if it should.
    pub fn exit_code(&mut self) -> Option<ExitCode> {
        self.exit.take()
    }

    /// Whether the user asked to end the session.
    pub fn is_quitting(&self) -> bool {
        self.lifecycle == Lifecycle::Quitting
    }

    /// Asks river for a manage sequence.
    fn dirty(&self) {
        if self.lifecycle == Lifecycle::Running {
            self.globals.wm.manage_dirty();
        }
    }

    /// Stops window management (`SIGTERM`, `SIGINT`); the loop exits on
    /// `finished`.
    pub fn stop(&mut self) {
        if self.lifecycle == Lifecycle::Running {
            tracing::info!("stopping");
            self.lifecycle = Lifecycle::Stopping;
            self.globals.wm.stop();
        }
    }

    /// Reloads the config in the next manage sequence (`SIGUSR1`).
    pub fn request_reload(&mut self) {
        self.reload = true;
        self.dirty();
    }

    /// Reaps exited children (`SIGCHLD`).
    pub fn reap_children(&mut self) {
        self.children.reap();
    }

    /// Updates the bar's status modules, redrawing if they changed.
    pub fn set_status(&mut self, update: impl FnOnce(&mut Status)) {
        let before = self.status.clone();
        update(&mut self.status);
        if self.status != before {
            self.dirty();
        }
    }

    /// Queues a command from the control socket.
    pub fn control(&mut self, line: &str, reply: Reply) {
        match control::parse(line) {
            Ok(command) => {
                self.inputs.push_back(Input::Control(command, reply));
                self.dirty();
            }
            Err(err) => reply.send(&format!("error: {err}")),
        }
    }

    fn bind_wl_output(&mut self, name: u32, version: u32) {
        let output: WlOutput =
            self.globals
                .list
                .registry()
                .bind(name, version.min(4), &self.qh, name);
        self.wl_outputs.insert(
            name,
            WlOutputInfo {
                output,
                name: String::new(),
                scale: 1,
            },
        );
    }

    // ----------------------------------------------------------------
    // Manage sequence

    fn manage_sequence(&mut self) {
        self.sync_outputs();
        self.sync_seats();
        self.sync_windows();

        let mut deferred = Vec::new();
        while let Some(input) = self.inputs.pop_front() {
            self.handle_input(input, &mut deferred);
        }
        self.answers.extend(deferred);

        if std::mem::take(&mut self.reload) {
            self.reload_config();
        }
        self.update_pointer_op();

        let arrangement = self.wm.arrange();
        let wanted = self.wanted_shells(&arrangement);
        let globals = ShellGlobals {
            wm: &self.globals.wm,
            compositor: &self.globals.compositor,
            shm: &self.globals.shm,
            qh: &self.qh,
        };
        self.shells.sync(wanted, &self.styles, &globals);
        let mut out = RiverRequests {
            windows: &self.windows,
            outputs: &self.outputs,
            seats: &self.seats,
            shells: &self.shells,
        };
        self.applied.manage(&arrangement, &mut out);
        self.arrangement = Some(arrangement);

        self.enable_bindings();
        self.set_default_layer_output();
        for seat in self.seats.values_mut() {
            seat.pointer_before = seat.pointer;
        }
        self.globals.wm.manage_finish();
    }

    /// Applies output changes to core.
    fn sync_outputs(&mut self) {
        let removed: Vec<ScreenId> = self
            .outputs
            .iter()
            .filter(|(_, o)| o.removed)
            .map(|(&id, _)| id)
            .collect();
        for id in removed {
            if let Some(output) = self.outputs.remove(&id) {
                self.wm.remove_screen(id);
                if let Some(layer) = output.layer {
                    layer.destroy();
                }
                output.proxy.destroy();
            }
        }
        for (&id, output) in &mut self.outputs {
            let Some(rect) = output.rect() else {
                continue;
            };
            if output.added && !output.changed {
                continue;
            }
            let name = output
                .global
                .and_then(|name| self.wl_outputs.get(&name))
                .map_or("", |info| info.name.as_str());
            self.wm.add_screen(id, name, rect);
            if let Some(usable) = output.usable {
                self.wm.set_usable_rect(id, usable);
            }
            output.added = true;
            output.changed = false;
        }
    }

    fn sync_seats(&mut self) {
        let removed: Vec<SeatId> = self
            .seats
            .iter()
            .filter(|(_, s)| s.removed)
            .map(|(&id, _)| id)
            .collect();
        for id in removed {
            if let Some(bindings) = self.bindings.remove(&id) {
                bindings.destroy();
            }
            if let Some(seat) = self.seats.remove(&id) {
                if let Some(layer) = seat.layer {
                    layer.destroy();
                }
                if let Some(keyboard) = seat.keyboard {
                    keyboard.release();
                }
                seat.proxy.destroy();
            }
        }
        for (&id, seat) in &mut self.seats {
            if !self.bindings.contains_key(&id) {
                self.bindings.insert(
                    id,
                    create_bindings(&self.globals, self.wm.config(), id, seat, &self.qh),
                );
            }
        }
    }

    /// Manages new windows, forgets closed ones and passes info changes.
    fn sync_windows(&mut self) {
        let closed: Vec<WindowId> = self
            .windows
            .iter()
            .filter(|(_, w)| w.closed)
            .map(|(&id, _)| id)
            .collect();
        for id in closed {
            if let Some(window) = self.windows.remove(&id) {
                self.wm.unmanage(id);
                self.applied.remove(id);
                window.node.destroy();
                window.proxy.destroy();
            }
            if self.op.as_ref().is_some_and(|op| op.window == id) {
                self.end_pointer_op();
            }
            for seat in self.seats.values_mut() {
                if seat.hovered == Some(id) {
                    seat.hovered = None;
                }
            }
        }
        for (&id, window) in &mut self.windows {
            if !window.managed {
                tracing::debug!(?id, app_id = %window.info.app_id, "manage");
                window.proxy.set_capabilities(Capabilities::Fullscreen);
                window.proxy.use_ssd();
                self.wm.manage(id, window.info.clone());
                window.managed = true;
                window.info_changed = false;
            } else if std::mem::take(&mut window.info_changed) {
                self.wm.update_info(id, window.info.clone());
            }
        }
    }

    fn handle_input(&mut self, input: Input, answers: &mut Vec<(Command, Reply)>) {
        match input {
            Input::Key(binding) => {
                if self.locked && binding.modified {
                    return;
                }
                self.apply(binding.action, None);
            }
            Input::Pointer(binding) => {
                if self.locked {
                    return;
                }
                // Like xmonad's mouse bindings: act on the window under
                // the pointer.
                if let Some(window) = self.seats.get(&binding.seat).and_then(|s| s.hovered) {
                    self.wm.focus_window(window);
                }
                self.apply(binding.action, Some(binding.seat));
            }
            Input::PromptKey(key) => {
                let effects = self.wm.prompt_key(key);
                self.execute(effects, None);
            }
            Input::Interaction(window) => self.wm.focus_window(window),
            Input::Hover(seat, window) => {
                let moved = self
                    .seats
                    .get(&seat)
                    .is_some_and(|s| s.pointer != s.pointer_before);
                if moved && self.wm.config().focus_follows_mouse {
                    self.wm.focus_window(window);
                }
            }
            Input::ShellClick(seat, node) => {
                let Some((x, y)) = self.seats.get(&seat).and_then(|s| s.pointer) else {
                    return;
                };
                match self.shells.click(node, x, y, &self.styles) {
                    Some(Click::Workspace(tag)) => self.apply(Action::View(tag), Some(seat)),
                    Some(Click::Window(window)) => self.wm.focus_window(window),
                    None => {}
                }
            }
            Input::MoveRequested(seat, window) => {
                self.wm.focus_window(window);
                self.apply(Action::FloatMove, Some(seat));
            }
            Input::ResizeRequested(seat, window) => {
                self.wm.focus_window(window);
                self.apply(Action::FloatResize, Some(seat));
            }
            Input::Fullscreen(window, on) => self.wm.request_fullscreen(window, on),
            Input::Control(command, reply) => {
                if let Command::Action(action) = &command {
                    self.apply(action.clone(), None);
                }
                answers.push((command, reply));
            }
        }
    }

    fn apply(&mut self, action: Action, seat: Option<SeatId>) {
        tracing::debug!(%action, "apply");
        let effects = self.wm.apply(action);
        self.execute(effects, seat);
    }

    fn execute(&mut self, effects: Vec<Effect>, seat: Option<SeatId>) {
        for effect in effects {
            match effect {
                Effect::Spawn(command) => self.children.spawn(&command),
                Effect::Close(window) => {
                    if let Some(window) = self.windows.get(&window) {
                        window.proxy.close();
                    }
                }
                Effect::Quit => self.quit(),
                Effect::Reload => self.reload = true,
                Effect::StartPointerMove(window) => {
                    self.start_pointer_op(window, OpKind::Move, seat);
                }
                Effect::StartPointerResize(window) => {
                    self.start_pointer_op(window, OpKind::Resize, seat);
                }
            }
        }
    }

    /// `quit`: ends the whole session, like xmonad's `Mod+Shift+q`.
    fn quit(&mut self) {
        tracing::info!("quit: exiting the session");
        self.lifecycle = Lifecycle::Quitting;
        if self.globals.wm.version() >= river_window_manager_v1::REQ_EXIT_SESSION_SINCE {
            self.globals.wm.exit_session();
        } else {
            self.stop();
        }
    }

    fn reload_config(&mut self) {
        match config_file::load(self.config_path.as_deref()) {
            Ok(config) => {
                tracing::info!("configuration reloaded");
                self.styles = Styles::new(&config, &mut self.fonts);
                self.wm.reconfigure(config);
                for (_, bindings) in std::mem::take(&mut self.bindings) {
                    bindings.destroy();
                }
                for (&id, seat) in &self.seats {
                    self.bindings.insert(
                        id,
                        create_bindings(&self.globals, self.wm.config(), id, seat, &self.qh),
                    );
                }
            }
            Err(err) => tracing::error!(%err, "keeping the previous configuration"),
        }
    }

    fn enable_bindings(&mut self) {
        for bindings in self.bindings.values_mut() {
            if !bindings.enabled {
                bindings.keys.iter().for_each(RiverXkbBindingV1::enable);
                bindings
                    .pointer
                    .iter()
                    .for_each(RiverPointerBindingV1::enable);
                bindings.enabled = true;
            }
        }
    }

    /// New layer surfaces without an output go to the current screen.
    fn set_default_layer_output(&self) {
        let current = self.wm.stack_set().current_screen().map(|s| s.id);
        if let Some(layer) = current
            .and_then(|id| self.outputs.get(&id))
            .and_then(|o| o.layer.as_ref())
        {
            layer.set_default();
        }
    }

    // ----------------------------------------------------------------
    // Pointer operations

    fn start_pointer_op(&mut self, window: WindowId, kind: OpKind, seat: Option<SeatId>) {
        if self.op.is_some() {
            return;
        }
        let Some(seat) = seat.or_else(|| self.seats.keys().next().copied()) else {
            return;
        };
        let Some(start) = self.wm.arrange().placement(window).map(|p| p.rect) else {
            return;
        };
        let (Some(seat_proxy), Some(window_proxy)) = (
            self.seats.get(&seat).map(|s| &s.proxy),
            self.windows.get(&window).map(|w| &w.proxy),
        ) else {
            return;
        };
        seat_proxy.op_start_pointer();
        if kind == OpKind::Resize {
            window_proxy.inform_resize_start();
        }
        self.op = Some(PointerOp {
            seat,
            window,
            kind,
            start,
            delta: None,
            released: false,
        });
    }

    fn update_pointer_op(&mut self) {
        let Some(op) = &mut self.op else {
            return;
        };
        if let Some((dx, dy)) = op.delta.take() {
            let rect = op_rect(op.kind, op.start, dx, dy);
            self.wm.set_float_rect(op.window, rect);
        }
        if op.released {
            self.end_pointer_op();
        }
    }

    fn end_pointer_op(&mut self) {
        let Some(op) = self.op.take() else {
            return;
        };
        if let Some(seat) = self.seats.get(&op.seat) {
            seat.proxy.op_end();
        }
        if op.kind == OpKind::Resize
            && let Some(window) = self.windows.get(&op.window)
        {
            window.proxy.inform_resize_end();
        }
    }

    // ----------------------------------------------------------------
    // Shell surfaces

    fn scale_of(&self, screen: ScreenId) -> u32 {
        self.outputs
            .get(&screen)
            .and_then(|o| o.global)
            .and_then(|name| self.wl_outputs.get(&name))
            .map_or(1, |info| info.scale.max(1).unsigned_abs())
    }

    fn wanted_shells(&self, arrangement: &Arrangement) -> Vec<Wanted> {
        let mut wanted = Vec::new();
        for screen in &arrangement.screens {
            let scale = self.scale_of(screen.screen);
            if let Some(rect) = screen.bar {
                wanted.push(Wanted {
                    node: Node::Bar,
                    rect,
                    scale,
                    content: Content::Bar(shell::bar_view(
                        &self.wm.bar_model(),
                        &self.status,
                        &self.styles,
                    )),
                });
            }
            if let Some(strip) = &screen.tab_strip
                && strip.rect.w > 0
                && strip.rect.h > 0
            {
                wanted.push(Wanted {
                    node: Node::Tabs(screen.screen),
                    rect: strip.rect,
                    scale,
                    content: shell::tabs_content(strip),
                });
            }
        }
        if let (Some((screen, rect)), Some(model)) = (arrangement.prompt, self.wm.prompt_model()) {
            let max_height = self
                .wm
                .stack_set()
                .screen(screen)
                .map_or(rect.h, |s| s.usable_rect.h);
            wanted.push(Wanted {
                node: Node::Prompt,
                rect,
                scale: self.scale_of(screen),
                content: Content::Prompt(shell::prompt_view(&model), max_height.unsigned_abs()),
            });
        }
        wanted
    }

    // ----------------------------------------------------------------
    // Render sequence

    fn render_sequence(&mut self) {
        if let Some(arrangement) = &self.arrangement {
            let placements = self.shells.placements(&self.styles);
            let mut out = RiverRequests {
                windows: &self.windows,
                outputs: &self.outputs,
                seats: &self.seats,
                shells: &self.shells,
            };
            let windows = &self.windows;
            plan::render(
                arrangement,
                |id| windows.get(&id).and_then(|w| w.dimensions),
                &placements,
                &mut out,
            );
        }
        self.globals.wm.render_finish();
        for (command, reply) in std::mem::take(&mut self.answers) {
            match command {
                Command::Action(_) => reply.send("ok"),
                Command::DumpState => reply.send(&self.dump_state()),
            }
        }
    }

    /// The state as one line of JSON (the control socket's `dump-state`).
    fn dump_state(&self) -> String {
        use serde_json::json;

        let rect = |r: Rect| json!([r.x, r.y, r.w, r.h]);
        let ss = self.wm.stack_set();
        let arrangement = self.arrangement.as_ref();
        let screens: Vec<_> = ss
            .screens()
            .iter()
            .map(|s| {
                let shells =
                    arrangement.and_then(|a| a.screens.iter().find(|sa| sa.screen == s.id));
                json!({
                    "id": s.id.0,
                    "name": s.name,
                    "rect": rect(s.rect),
                    "usable": rect(s.usable_rect),
                    "workspace": s.workspace.tag,
                    "layout": s.workspace.layout.description(),
                    "bar": shells.and_then(|sa| sa.bar).map(rect),
                    "tabs": shells.and_then(|sa| sa.tab_strip.as_ref()).map(|t| json!({
                        "rect": rect(t.rect),
                        "titles": t.tabs.iter().map(|tab| tab.title.clone()).collect::<Vec<_>>(),
                    })),
                })
            })
            .collect();
        let windows: Vec<_> = arrangement
            .map(|a| a.windows.as_slice())
            .unwrap_or_default()
            .iter()
            .map(|p| {
                let window = self.windows.get(&p.window);
                let info = self.wm.window_info(p.window);
                let dimensions = window.and_then(|w| w.dimensions);
                let (x, y) = plan::window_position(p, dimensions);
                json!({
                    "id": p.window.0,
                    "app_id": info.map(|i| i.app_id.clone()),
                    "title": info.map(|i| i.title.clone()),
                    "screen": p.screen.0,
                    "layer": format!("{:?}", p.layer).to_lowercase(),
                    "rect": rect(p.rect),
                    "position": [x, y],
                    "size": dimensions.map(|d| [d.w, d.h]),
                    "border": p.border.map(|b| b.width),
                })
            })
            .collect();
        let focus = match arrangement.map(|a| a.focus) {
            Some(Focus::Window(id)) => json!(id.0),
            Some(Focus::Prompt) => json!("prompt"),
            Some(Focus::None) | None => json!(null),
        };
        json!({
            "current_workspace": ss.current_tag(),
            "screens": screens,
            "windows": windows,
            "hidden": arrangement.map(|a| a.hidden.iter().map(|w| w.0).collect::<Vec<_>>()),
            "focus": focus,
            "prompt": self.wm.prompt_model().map(|m| json!({
                "label": m.label,
                "input": m.input,
                "completions": m.completions,
            })),
            "shells": self.shells.placements(&self.styles).iter().map(|s| json!({
                "node": format!("{:?}", s.node),
                "rect": rect(s.rect),
            })).collect::<Vec<_>>(),
            "bar_clicks": self
                .shells
                .click_targets(Node::Bar, &self.styles)
                .into_iter()
                .filter_map(|(start, end, click)| match click {
                    Click::Workspace(tag) => Some(json!({ "workspace": tag, "x": [start, end] })),
                    Click::Window(_) => None,
                })
                .collect::<Vec<_>>(),
            "bar_visible": self.wm.bar_visible(),
            "managed": self.windows.len(),
        })
        .to_string()
    }
}

/// The content rectangle of a window being moved or resized by `(dx, dy)`.
#[must_use]
fn op_rect(kind: OpKind, start: Rect, dx: i32, dy: i32) -> Rect {
    match kind {
        OpKind::Move => Rect::new(start.x + dx, start.y + dy, start.w, start.h),
        OpKind::Resize => Rect::new(
            start.x,
            start.y,
            (start.w + dx).max(1),
            (start.h + dy).max(1),
        ),
    }
}

fn create_bindings(
    globals: &Globals,
    config: &Config,
    id: SeatId,
    seat: &Seat,
    qh: &QueueHandle<State>,
) -> SeatBindings {
    let mut keys_out = Vec::new();
    if let Some(xkb) = &globals.xkb {
        for binding in &config.keybindings {
            let Some(resolved) = keys::resolve(&binding.combo) else {
                tracing::warn!(combo = %binding.combo, "unknown key name, binding ignored");
                continue;
            };
            keys_out.push(xkb.get_xkb_binding(
                &seat.proxy,
                resolved.keysym,
                river_seat_v1::Modifiers::from_bits_truncate(resolved.modifiers),
                qh,
                KeyBindingData {
                    action: binding.action.clone(),
                    modified: !binding.combo.mods.is_empty(),
                },
            ));
        }
    } else {
        tracing::warn!("river_xkb_bindings_v1 is missing: no key bindings");
    }
    let pointer = config
        .pointer_bindings
        .iter()
        .map(|binding| {
            seat.proxy.get_pointer_binding(
                binding.combo.button.evdev_code(),
                river_seat_v1::Modifiers::from_bits_truncate(keys::river_modifiers(
                    binding.combo.mods,
                )),
                qh,
                PointerBindingData {
                    action: binding.action.clone(),
                    seat: id,
                },
            )
        })
        .collect();
    SeatBindings {
        keys: keys_out,
        pointer,
        enabled: false,
    }
}

/// [`Requests`] sent to river.
struct RiverRequests<'a> {
    windows: &'a BTreeMap<WindowId, Window>,
    outputs: &'a BTreeMap<ScreenId, Output>,
    seats: &'a BTreeMap<SeatId, Seat>,
    shells: &'a Shells,
}

impl RiverRequests<'_> {
    fn window(&self, id: WindowId) -> Option<&RiverWindowV1> {
        self.windows.get(&id).map(|w| &w.proxy)
    }
}

impl Requests for RiverRequests<'_> {
    fn propose_dimensions(&mut self, window: WindowId, size: Size) {
        if let Some(w) = self.window(window) {
            w.propose_dimensions(size.w, size.h);
        }
    }

    fn set_tiled(&mut self, window: WindowId, tiled: bool) {
        if let Some(w) = self.window(window) {
            w.set_tiled(if tiled { Edges::all() } else { Edges::empty() });
        }
    }

    fn fullscreen(&mut self, window: WindowId, screen: ScreenId) {
        if let (Some(w), Some(output)) = (self.window(window), self.outputs.get(&screen)) {
            w.fullscreen(&output.proxy);
            w.inform_fullscreen();
        }
    }

    fn exit_fullscreen(&mut self, window: WindowId) {
        if let Some(w) = self.window(window) {
            w.exit_fullscreen();
            w.inform_not_fullscreen();
        }
    }

    fn focus(&mut self, focus: Focus) {
        for seat in self.seats.values().filter(|s| !s.removed) {
            match focus {
                Focus::Window(id) => match self.window(id) {
                    Some(w) => seat.proxy.focus_window(w),
                    None => seat.proxy.clear_focus(),
                },
                Focus::Prompt => match self.shells.shell_surface(Node::Prompt) {
                    Some(prompt) => seat.proxy.focus_shell_surface(prompt),
                    None => seat.proxy.clear_focus(),
                },
                Focus::None => seat.proxy.clear_focus(),
            }
        }
    }

    fn show(&mut self, window: WindowId) {
        if let Some(w) = self.window(window) {
            w.show();
        }
    }

    fn hide(&mut self, window: WindowId) {
        if let Some(w) = self.window(window) {
            w.hide();
        }
    }

    fn set_borders(&mut self, window: WindowId, border: Option<Border>) {
        let Some(w) = self.window(window) else {
            return;
        };
        match border {
            Some(Border { width, color }) => w.set_borders(
                Edges::all(),
                width,
                plan::border_channel(color.r, color.a),
                plan::border_channel(color.g, color.a),
                plan::border_channel(color.b, color.a),
                plan::border_channel(color.a, 0xff),
            ),
            None => w.set_borders(Edges::empty(), 0, 0, 0, 0, 0),
        }
    }

    fn set_position(&mut self, node: Node, x: i32, y: i32) {
        let proxy = match node {
            Node::Window(id) => self.windows.get(&id).map(|w| &w.node),
            Node::Bar | Node::Tabs(_) | Node::Prompt => self.shells.node_proxy(node),
        };
        if let Some(proxy) = proxy {
            proxy.set_position(x, y);
        }
    }

    fn place_top(&mut self, node: Node) {
        let proxy = match node {
            Node::Window(id) => self.windows.get(&id).map(|w| &w.node),
            Node::Bar | Node::Tabs(_) | Node::Prompt => self.shells.node_proxy(node),
        };
        if let Some(proxy) = proxy {
            proxy.place_top();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pointer_op_rects() {
        let start = Rect::new(100, 100, 300, 200);
        assert_eq!(
            op_rect(OpKind::Move, start, 10, -20),
            Rect::new(110, 80, 300, 200)
        );
        assert_eq!(
            op_rect(OpKind::Resize, start, 10, -20),
            Rect::new(100, 100, 310, 180)
        );
        assert_eq!(
            op_rect(OpKind::Resize, start, -400, -400),
            Rect::new(100, 100, 1, 1)
        );
    }
}
