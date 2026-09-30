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

use calloop::{
    LoopHandle,
    RegistrationToken,
    timer::{
        TimeoutAction,
        Timer,
    },
};
use oxmonad_core::{
    Action,
    Arrangement,
    Border,
    Effect,
    Focus,
    OutputId,
    Rect,
    ScreenId,
    Size,
    Snapshot,
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
        wl_pointer::WlPointer,
        wl_seat::WlSeat,
        wl_shm::WlShm,
        wl_surface::WlSurface,
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
    keyboard::{
        Keyboard,
        RepeatTiming,
    },
    keys,
    plan::{
        self,
        Applied,
        Node,
        Requests,
    },
    restart::{
        self,
        Exe,
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
    /// The `wl_output`s and their global names, bound before `wm` (see
    /// [`bind_wl_outputs`]).
    pub wl_outputs: Vec<(u32, WlOutput)>,
}

/// Binds every advertised `wl_output`. Called before binding the window
/// manager, so the output names arrive before river announces its outputs:
/// a restart matches screens by name in the first manage sequence.
pub fn bind_wl_outputs(list: &GlobalList, qh: &QueueHandle<State>) -> Vec<(u32, WlOutput)> {
    let outputs: Vec<(u32, u32)> = list.contents().with_list(|list| {
        list.iter()
            .filter(|g| g.interface == WlOutput::interface().name)
            .map(|g| (g.name, g.version))
            .collect()
    });
    outputs
        .into_iter()
        .map(|(name, version)| (name, list.registry().bind(name, version.min(4), qh, name)))
        .collect()
}

#[derive(Debug)]
struct Window {
    proxy: RiverWindowV1,
    node: RiverNodeV1,
    /// river's stable name for the window, kept across restarts.
    identifier: Option<String>,
    info: WindowInfo,
    /// Dimensions river reported.
    dimensions: Option<Size>,
    managed: bool,
    info_changed: bool,
    closed: bool,
}

impl Window {
    fn destroy(self) {
        self.node.destroy();
        self.proxy.destroy();
    }
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

    fn destroy(self) {
        if let Some(layer) = self.layer {
            layer.destroy();
        }
        self.proxy.destroy();
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
    /// The WM's own pointer, for hovering over its shell surfaces.
    wl_pointer: Option<WlPointer>,
    /// The shell surface under that pointer.
    pointer_surface: Option<WlSurface>,
    /// The layout picker entry last reported as hovered.
    hovered_layout: Option<usize>,
    pointer: Option<(i32, i32)>,
    /// The pointer position in the previous manage sequence.
    pointer_before: Option<(i32, i32)>,
    hovered: Option<WindowId>,
    removed: bool,
}

impl Seat {
    fn destroy(self) {
        if let Some(layer) = self.layer {
            layer.destroy();
        }
        if let Some(keyboard) = self.keyboard {
            keyboard.release();
        }
        if let Some(pointer) = self.wl_pointer
            && pointer.version() >= 3
        {
            pointer.release();
        }
        if let Some(wl) = self.wl
            && wl.version() >= 5
        {
            wl.release();
        }
        self.proxy.destroy();
    }
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
    /// The pointer moved onto a layout picker entry.
    PickerHover(usize),
    /// An action from outside, e.g. `restart` on `SIGUSR2`.
    Action(Action),
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
    /// Sent `stop` to restart, waiting for `finished`.
    Restarting,
    /// Asked river to end the session.
    Quitting,
}

/// Everything the event loop works on.
#[derive(Debug)]
pub struct State {
    handle: LoopHandle<'static, Self>,
    /// The prompt key being repeated and its timer.
    repeat: Option<(u32, RegistrationToken)>,
    conn: Connection,
    qh: QueueHandle<Self>,
    globals: Globals,
    wm: Wm,
    config_path: Option<PathBuf>,
    fonts: FontCache,
    styles: Styles,
    status: Status,

    windows: BTreeMap<WindowId, Window>,
    outputs: BTreeMap<OutputId, Output>,
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
    /// How to start this binary again, for `restart`.
    exe: Option<Exe>,
    /// `restart` was applied in the running manage sequence.
    restart_requested: bool,
    /// The state to restore after restarting.
    restart: Option<Snapshot>,
    lifecycle: Lifecycle,
    exit: Option<ExitCode>,
}

impl State {
    /// A fresh state around the bound globals; `restore` is the state a
    /// restarting oxmonad left.
    pub fn new(
        handle: LoopHandle<'static, Self>,
        conn: Connection,
        qh: QueueHandle<Self>,
        mut globals: Globals,
        (config, config_path): (Config, Option<PathBuf>),
        exe: Option<Exe>,
        restore: Option<Snapshot>,
    ) -> Self {
        let mut fonts = FontCache::default();
        let styles = Styles::new(&config, &mut fonts);
        let mut wm = Wm::new(config);
        if let Some(snapshot) = restore {
            match wm.restore(snapshot) {
                Ok(()) => tracing::info!("restoring the state from before the restart"),
                Err(err) => tracing::warn!(%err, "starting fresh"),
            }
        }
        let wl_outputs = std::mem::take(&mut globals.wl_outputs)
            .into_iter()
            .map(|(name, output)| {
                let info = WlOutputInfo {
                    output,
                    name: String::new(),
                    scale: 1,
                };
                (name, info)
            })
            .collect();
        Self {
            handle,
            repeat: None,
            conn,
            qh,
            globals,
            wm,
            config_path,
            fonts,
            styles,
            status: Status::default(),
            windows: BTreeMap::new(),
            outputs: BTreeMap::new(),
            wl_outputs,
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
            exe,
            restart_requested: false,
            restart: None,
            lifecycle: Lifecycle::Running,
            exit: None,
        }
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

    /// Restarts in place in the next manage sequence (`SIGUSR2`, sent by
    /// `oxmonad --restart`).
    pub fn request_restart(&mut self) {
        self.inputs.push_back(Input::Action(Action::Restart));
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

    /// Repeats `key` (typed into the prompt) while it is held.
    fn start_repeat(
        &mut self,
        seat: SeatId,
        key: u32,
        prompt_key: PromptKey,
        timing: RepeatTiming,
    ) {
        self.stop_repeat(None);
        let timer = Timer::from_duration(timing.delay);
        let token = self.handle.insert_source(timer, move |_, (), state| {
            let held = state.seats.contains_key(&seat) && state.wm.grabs_keyboard();
            if !held {
                state.repeat = None;
                return TimeoutAction::Drop;
            }
            state.inputs.push_back(Input::PromptKey(prompt_key));
            state.dirty();
            TimeoutAction::ToDuration(timing.interval)
        });
        match token {
            Ok(token) => self.repeat = Some((key, token)),
            Err(err) => tracing::warn!(%err, "cannot repeat keys"),
        }
    }

    /// Stops repeating (`key`, or whatever repeats).
    fn stop_repeat(&mut self, key: Option<u32>) {
        if let Some((repeating, token)) = self.repeat
            && key.is_none_or(|key| key == repeating)
        {
            self.handle.remove(token);
            self.repeat = None;
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
        // Before handling input, so a key pressed after moving the pointer
        // to another screen acts on that screen.
        self.follow_pointer_screen();

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
        self.update_help_viewport(&arrangement);
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
        if std::mem::take(&mut self.restart_requested) {
            self.begin_restart();
        }
    }

    /// Makes the screen under a moved pointer current (focus follows
    /// mouse across screens). river reports the pointer only in manage
    /// sequences, so this takes effect at the next one, e.g. the key press
    /// that follows the move.
    fn follow_pointer_screen(&mut self) {
        if !self.wm.config().focus_follows_mouse || self.op.is_some() {
            return;
        }
        let moved = self.seats.values().find_map(|seat| {
            seat.pointer
                .filter(|_| seat.pointer_before.is_some() && seat.pointer != seat.pointer_before)
        });
        if let Some((x, y)) = moved {
            self.wm.focus_screen_at(x, y);
        }
    }

    /// Applies output changes to core.
    fn sync_outputs(&mut self) {
        let removed: Vec<OutputId> = self
            .outputs
            .iter()
            .filter(|(_, o)| o.removed)
            .map(|(&id, _)| id)
            .collect();
        for id in removed {
            if let Some(output) = self.outputs.remove(&id) {
                self.wm.remove_output(id);
                output.destroy();
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
            self.wm.add_output(id, name, rect);
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
                seat.destroy();
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
                window.destroy();
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
                self.wm
                    .manage_as(id, window.identifier.as_deref(), window.info.clone());
                window.managed = true;
                window.info_changed = false;
            } else if std::mem::take(&mut window.info_changed) {
                self.wm.update_info(id, window.info.clone());
            }
        }
        // river announces every existing window before the first manage
        // sequence, so a restore is complete now.
        self.wm.finish_restore();
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
            Input::Action(action) => self.apply(action, None),
            Input::Hover(seat, window) => {
                // The first position river reports (e.g. right after a
                // restart) is not a movement.
                let moved = self
                    .seats
                    .get(&seat)
                    .is_some_and(|s| s.pointer_before.is_some() && s.pointer != s.pointer_before);
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
                    Some(Click::Layout(index)) => self.wm.pick_layout(index),
                    Some(Click::Dismiss) => self.wm.close_help(),
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
            Input::PickerHover(index) => self.wm.hover_layout(index),
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
                Effect::Restart => self.restart_requested = true,
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

    // ----------------------------------------------------------------
    // Restart

    /// `restart`: if the binary still works, saves the state and stops
    /// window management; [`Self::finish_restart`] re-executes on
    /// `finished`.
    fn begin_restart(&mut self) {
        if self.lifecycle != Lifecycle::Running {
            return;
        }
        let Some(exe) = &self.exe else {
            tracing::error!("restart: the oxmonad binary is unknown");
            return;
        };
        if let Err(err) = exe.check() {
            tracing::error!("restart: {err}; keeping this oxmonad");
            return;
        }
        let windows = &self.windows;
        let snapshot = self
            .wm
            .snapshot(|id| windows.get(&id).and_then(|w| w.identifier.clone()));
        tracing::info!("restarting");
        self.restart = Some(snapshot);
        self.lifecycle = Lifecycle::Restarting;
        self.globals.wm.stop();
    }

    /// Whether `finished` belongs to a restart.
    fn is_restarting(&self) -> bool {
        self.lifecycle == Lifecycle::Restarting
    }

    /// river finished with the window manager during a restart: releases
    /// every window-management object and re-executes the binary. If that
    /// fails, binds the window manager again and restores the snapshot in
    /// this process.
    fn finish_restart(&mut self) {
        let Some(snapshot) = self.restart.take() else {
            return;
        };
        for (command, reply) in std::mem::take(&mut self.answers) {
            match command {
                Command::Action(_) => reply.send("ok"),
                Command::DumpState => reply.send(&self.dump_state()),
            }
        }
        self.teardown();
        self.flush();

        let saved = restart::save(&snapshot);
        match (&self.exe, saved) {
            (Some(exe), Ok(path)) => {
                let err = exe.exec(&path);
                tracing::error!(%err, "restart: cannot execute the oxmonad binary");
                restart::discard(&path);
            }
            (_, Err(err)) => tracing::error!(%err, "restart: cannot save the state"),
            (None, Ok(path)) => restart::discard(&path),
        }
        self.resume(snapshot);
    }

    /// Destroys every object created through the window manager and
    /// forgets river's windows, outputs and seats.
    fn teardown(&mut self) {
        self.stop_repeat(None);
        self.op = None;
        self.inputs.clear();
        for (_, bindings) in std::mem::take(&mut self.bindings) {
            bindings.destroy();
        }
        for (_, seat) in std::mem::take(&mut self.seats) {
            seat.destroy();
        }
        for (_, output) in std::mem::take(&mut self.outputs) {
            output.destroy();
        }
        for (_, window) in std::mem::take(&mut self.windows) {
            window.destroy();
        }
        self.shells.destroy();
        self.applied = Applied::default();
        self.arrangement = None;
        self.globals.wm.destroy();
    }

    /// Continues in this process after a failed restart: binds the window
    /// manager again (river announces everything anew) and restores
    /// `snapshot` into a fresh model with the configuration reloaded.
    fn resume(&mut self, snapshot: Snapshot) {
        let config = match config_file::load(self.config_path.as_deref()) {
            Ok(config) => {
                self.styles = Styles::new(&config, &mut self.fonts);
                config
            }
            Err(err) => {
                tracing::error!(%err, "keeping the previous configuration");
                self.wm.config().clone()
            }
        };
        let mut wm = Wm::new(config);
        if let Err(err) = wm.restore(snapshot) {
            tracing::warn!(%err, "cannot restore the state");
        }
        self.wm = wm;
        match self.globals.list.bind(&self.qh, 1..=5, ()) {
            Ok(wm) => {
                tracing::info!("window management resumed");
                self.globals.wm = wm;
                self.lifecycle = Lifecycle::Running;
            }
            Err(err) => {
                tracing::error!(%err, "cannot bind the window manager again");
                self.exit = Some(ExitCode::FAILURE);
            }
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
        let current = self.wm.stack_set().current_screen().map(|s| s.output);
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

    fn scale_of(&self, output: OutputId) -> u32 {
        self.outputs
            .get(&output)
            .and_then(|o| o.global)
            .and_then(|name| self.wl_outputs.get(&name))
            .map_or(1, |info| info.scale.max(1).unsigned_abs())
    }

    /// Tells core how many rows of the open help overlay fit its screen,
    /// so scrolling stops at the end.
    fn update_help_viewport(&mut self, arrangement: &Arrangement) {
        let (Some((screen, area)), Some(model)) = (arrangement.help, self.wm.help_model()) else {
            return;
        };
        let scale = self.screen_scale(screen);
        let (page, max_scroll) =
            shell::help_rows(&shell::help_view(&model), area, scale, &self.styles);
        self.wm.set_help_viewport(page, max_scroll);
    }

    /// The buffer scale of `screen`'s output.
    fn screen_scale(&self, screen: ScreenId) -> u32 {
        self.wm
            .stack_set()
            .screen(screen)
            .map_or(1, |s| self.scale_of(s.output))
    }

    fn wanted_shells(&self, arrangement: &Arrangement) -> Vec<Wanted> {
        let mut wanted = Vec::new();
        if let Some((output, rect)) = arrangement.bar {
            wanted.push(Wanted {
                node: Node::Bar,
                rect,
                scale: self.scale_of(output),
                content: Content::Bar(shell::bar_view(
                    &self.wm.bar_model(),
                    &self.status,
                    &self.styles,
                )),
            });
        }
        for screen in &arrangement.screens {
            let scale = self.scale_of(screen.output);
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
            let screen = self.wm.stack_set().screen(screen);
            let max_height = screen.map_or(rect.h, |s| s.usable_rect.h);
            wanted.push(Wanted {
                node: Node::Prompt,
                rect,
                scale: screen.map_or(1, |s| self.scale_of(s.output)),
                content: Content::Prompt(shell::prompt_view(&model), max_height.unsigned_abs()),
            });
        }
        if let (Some((screen, area)), Some(model)) =
            (arrangement.layout_picker, self.wm.layout_picker_model())
        {
            let scale = self
                .wm
                .stack_set()
                .screen(screen)
                .map_or(1, |s| self.scale_of(s.output));
            wanted.push(Wanted {
                node: Node::LayoutPicker,
                rect: area,
                scale,
                content: Content::LayoutPicker(shell::layout_picker_view(&model)),
            });
        }
        if let (Some((screen, area)), Some(model)) = (arrangement.help, self.wm.help_model()) {
            wanted.push(Wanted {
                node: Node::Help,
                rect: area,
                scale: self.screen_scale(screen),
                content: Content::Help(shell::help_view(&model)),
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

    /// The open layout picker for `dump-state`: entries, selection and the
    /// cards' global rectangles.
    fn layout_picker_state(&self) -> serde_json::Value {
        use serde_json::json;

        let Some(model) = self.wm.layout_picker_model() else {
            return serde_json::Value::Null;
        };
        json!({
            "workspace": model.tag,
            "entries": model.entries.iter().map(|e| e.name.clone()).collect::<Vec<_>>(),
            "selected": model.selected,
            "active": model.active,
            "cards": self
                .shells
                .layout_picker_cards(&self.styles)
                .into_iter()
                .map(|r| json!([r.x, r.y, r.w, r.h]))
                .collect::<Vec<_>>(),
        })
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
                    "output": s.output.0,
                    "name": s.name,
                    "split": ss.is_split(s.output),
                    "rect": rect(s.rect),
                    "usable": rect(s.usable_rect),
                    "workspace": s.workspace.tag,
                    "layout": s.workspace.layout.name(),
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
                    "identifier": window.and_then(|w| w.identifier.clone()),
                    "app_id": info.map(|i| i.app_id.clone()),
                    "title": info.map(|i| i.title.clone()),
                    "screen": p.screen.0,
                    "output": p.output.0,
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
            Some(Focus::LayoutPicker) => json!("layout-picker"),
            Some(Focus::Help) => json!("help"),
            Some(Focus::None) | None => json!(null),
        };
        json!({
            "current_workspace": ss.current_tag(),
            "screens": screens,
            "bar": arrangement.and_then(|a| a.bar).map(|(output, r)| json!({
                "output": output.0,
                "name": ss.output(output).map(|o| o.name.clone()),
                "rect": rect(r),
            })),
            "windows": windows,
            "hidden": arrangement.map(|a| a.hidden.iter().map(|w| w.0).collect::<Vec<_>>()),
            "focus": focus,
            "prompt": self.wm.prompt_model().map(|m| json!({
                "label": m.label,
                "input": m.input,
                "completions": m.completions,
            })),
            "layout_picker": self.layout_picker_state(),
            "help": self.wm.help_model().map(|m| json!({
                "scroll": m.scroll,
                "max_scroll": m.max_scroll,
                "sections": m.groups.iter().map(|g| g.section.title()).collect::<Vec<_>>(),
                "bindings": m.groups.iter().map(|g| g.entries.len()).sum::<usize>(),
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
                    Click::Window(_) | Click::Layout(_) | Click::Dismiss => None,
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
    outputs: &'a BTreeMap<OutputId, Output>,
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

    fn fullscreen(&mut self, window: WindowId, output: OutputId) {
        if let (Some(w), Some(output)) = (self.window(window), self.outputs.get(&output)) {
            w.fullscreen(&output.proxy);
        }
    }

    fn exit_fullscreen(&mut self, window: WindowId) {
        if let Some(w) = self.window(window) {
            w.exit_fullscreen();
        }
    }

    fn inform_fullscreen(&mut self, window: WindowId, fullscreen: bool) {
        if let Some(w) = self.window(window) {
            if fullscreen {
                w.inform_fullscreen();
            } else {
                w.inform_not_fullscreen();
            }
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
                Focus::LayoutPicker => match self.shells.shell_surface(Node::LayoutPicker) {
                    Some(picker) => seat.proxy.focus_shell_surface(picker),
                    None => seat.proxy.clear_focus(),
                },
                Focus::Help => match self.shells.shell_surface(Node::Help) {
                    Some(help) => seat.proxy.focus_shell_surface(help),
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
            Node::Bar | Node::Tabs(_) | Node::Prompt | Node::LayoutPicker | Node::Help => {
                self.shells.node_proxy(node)
            }
        };
        if let Some(proxy) = proxy {
            proxy.set_position(x, y);
        }
    }

    fn place_top(&mut self, node: Node) {
        let proxy = match node {
            Node::Window(id) => self.windows.get(&id).map(|w| &w.node),
            Node::Bar | Node::Tabs(_) | Node::Prompt | Node::LayoutPicker | Node::Help => {
                self.shells.node_proxy(node)
            }
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
