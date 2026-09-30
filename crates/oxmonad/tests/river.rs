//! End-to-end test: oxmonad as the window manager of a headless river.
//!
//! Ignored by default; run with
//! `cargo test -p oxmonad --test river -- --ignored`. Needs `river` (0.4+)
//! and `alacritty` on `PATH` and skips itself otherwise. Everything runs in
//! a private `XDG_RUNTIME_DIR` under `$TMPDIR` (keep that path short: Unix
//! socket paths are limited to 108 bytes).
//!
//! The test drives oxmonad through its control socket (`OXMONAD_CONTROL`)
//! and a virtual keyboard, and checks the state oxmonad reports with
//! `dump-state`.

use std::{
    env,
    fs::{
        self,
        File,
    },
    io::{
        BufRead,
        BufReader,
        Write,
    },
    os::{
        fd::AsFd,
        unix::net::UnixStream,
    },
    path::{
        Path,
        PathBuf,
    },
    process::{
        Child,
        Command,
        Stdio,
    },
    thread::sleep,
    time::{
        Duration,
        Instant,
    },
};

use kbvm::xkb::{
    Context,
    diagnostic::Diagnostic,
    rmlvo::Group,
};
use serde_json::Value;
use wayland_client::{
    Connection,
    Dispatch,
    QueueHandle,
    globals::{
        GlobalListContents,
        registry_queue_init,
    },
    protocol::{
        wl_registry::WlRegistry,
        wl_seat::WlSeat,
    },
};
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::{
    zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1,
    zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1,
};
use wayland_protocols_wlr::virtual_pointer::v1::client::{
    zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1,
    zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1,
};

/// Linux evdev key codes.
mod key {
    pub const ESC: u32 = 1;
    pub const THREE: u32 = 4;
    pub const J: u32 = 36;
    pub const K: u32 = 37;
    pub const V: u32 = 47;
    pub const ENTER: u32 = 28;
    pub const BACKSPACE: u32 = 14;
    pub const LEFT_META: u32 = 125;
    pub const LEFT_SHIFT: u32 = 42;
    pub const SLASH: u32 = 53;
    pub const Q: u32 = 16;
    pub const PAGE_DOWN: u32 = 109;
    pub const BTN_LEFT: u32 = 0x110;
}

/// The xkb modifier mask of Mod4 in the `us` keymap.
const MOD4: u32 = 1 << 6;
const SHIFT: u32 = 1;

fn have(program: &str) -> bool {
    Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {program}"))
        .stdout(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Kills every process it started when dropped, then removes the runtime
/// directory.
struct Session {
    dir: PathBuf,
    river: Child,
    clients: Vec<Child>,
}

impl Drop for Session {
    fn drop(&mut self) {
        for child in &mut self.clients {
            let _ = child.kill();
            let _ = child.wait();
        }
        let _ = self.river.kill();
        let _ = self.river.wait();
        let _ = fs::remove_dir_all(&self.dir);
    }
}

impl Session {
    /// A private runtime directory `oxm-<pid>-<label>` (with a `config`
    /// directory for `XDG_CONFIG_HOME`).
    fn dir(label: &str) -> PathBuf {
        let dir = env::temp_dir().join(format!("oxm-{}-{label}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("config")).expect("runtime dir");
        dir
    }

    fn start() -> Self {
        Self::start_in(Self::dir("e2e"), Path::new(env!("CARGO_BIN_EXE_oxmonad")))
    }

    /// Starts river in `dir` (see [`Self::dir`]) with `binary` as its
    /// window manager.
    fn start_in(dir: PathBuf, binary: &Path) -> Self {
        let log = File::create(dir.join("river.log")).expect("log file");
        let init = binary.to_str().expect("UTF-8 binary path");
        let river = Command::new("river")
            .args(["-no-xwayland", "-c", init])
            .env_remove("WAYLAND_DISPLAY")
            .env_remove("WAYLAND_SOCKET")
            .env_remove("DISPLAY")
            .env("XDG_RUNTIME_DIR", &dir)
            .env("XDG_CONFIG_HOME", dir.join("config"))
            .env("WLR_BACKENDS", "headless")
            .env("WLR_RENDERER", "pixman")
            .env("WLR_HEADLESS_OUTPUTS", "1")
            .env("OXMONAD_CONTROL", dir.join("ctl"))
            .env("RUST_LOG", "oxmonad=debug")
            .stdout(log.try_clone().expect("log file"))
            .stderr(log)
            .spawn()
            .expect("river starts");
        let session = Self {
            dir,
            river,
            clients: Vec::new(),
        };
        wait_for("the control socket", || session.control_path().exists());
        session
    }

    fn control_path(&self) -> PathBuf {
        self.dir.join("ctl")
    }

    fn wayland_display(&self) -> String {
        fs::read_dir(&self.dir)
            .expect("runtime dir")
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .find(|name| name.starts_with("wayland-") && !name.contains('.'))
            .expect("river's Wayland socket")
    }

    fn command(&self, line: &str) -> String {
        self.try_command(line)
            .unwrap_or_else(|| panic!("no answer to {line}"))
    }

    /// Sends `line`; `None` if oxmonad does not answer (e.g. while it
    /// restarts).
    fn try_command(&self, line: &str) -> Option<String> {
        let mut stream = UnixStream::connect(self.control_path()).ok()?;
        stream.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
        writeln!(stream, "{line}").ok()?;
        let mut answer = String::new();
        BufReader::new(stream).read_line(&mut answer).ok()?;
        Some(answer.trim().to_owned()).filter(|answer| !answer.is_empty())
    }

    /// `dump-state`, or `None` if oxmonad does not answer.
    fn try_state(&self) -> Option<Value> {
        serde_json::from_str(&self.try_command("dump-state")?).ok()
    }

    fn act(&self, action: &str) {
        assert_eq!(self.command(action), "ok", "{action}");
    }

    fn state(&self) -> Value {
        serde_json::from_str(&self.command("dump-state")).expect("dump-state is JSON")
    }

    fn spawn_terminal(&mut self, title: &str) {
        self.spawn_terminal_with(title, &[]);
    }

    /// Opens a terminal titled `title` with extra alacritty arguments.
    fn spawn_terminal_with(&mut self, title: &str, args: &[&str]) {
        let child = Command::new("alacritty")
            .args(args)
            .args(["-T", title, "-e", "sleep", "1000"])
            .env_remove("DISPLAY")
            .env("XDG_RUNTIME_DIR", &self.dir)
            .env("WAYLAND_DISPLAY", self.wayland_display())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("alacritty starts");
        self.clients.push(child);
    }

    fn log(&self) -> String {
        fs::read_to_string(self.dir.join("river.log")).unwrap_or_default()
    }
}

fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        sleep(Duration::from_millis(50));
    }
}

fn windows(state: &Value) -> Vec<Value> {
    state["windows"].as_array().cloned().unwrap_or_default()
}

fn rect(value: &Value) -> [i64; 4] {
    let v: Vec<i64> = value
        .as_array()
        .expect("rect")
        .iter()
        .map(|n| n.as_i64().expect("int"))
        .collect();
    [v[0], v[1], v[2], v[3]]
}

fn overlap(a: [i64; 4], b: [i64; 4]) -> bool {
    a[0] < b[0] + b[2] && b[0] < a[0] + a[2] && a[1] < b[1] + b[3] && b[1] < a[1] + a[3]
}

/// A virtual keyboard and pointer on river's seat.
struct Input {
    conn: Connection,
    keyboard: ZwpVirtualKeyboardV1,
    pointer: ZwlrVirtualPointerV1,
    /// The output size, for absolute pointer motion.
    extent: (u32, u32),
    time: u32,
}

struct Client;

impl Dispatch<WlRegistry, GlobalListContents> for Client {
    fn event(
        _: &mut Self,
        _: &WlRegistry,
        _: wayland_client::protocol::wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

wayland_client::delegate_noop!(Client: ignore WlSeat);
wayland_client::delegate_noop!(Client: ZwpVirtualKeyboardManagerV1);
wayland_client::delegate_noop!(Client: ZwpVirtualKeyboardV1);
wayland_client::delegate_noop!(Client: ZwlrVirtualPointerManagerV1);
wayland_client::delegate_noop!(Client: ZwlrVirtualPointerV1);

impl Input {
    fn connect(session: &Session, extent: (u32, u32)) -> Self {
        let socket = session.dir.join(session.wayland_display());
        let stream = UnixStream::connect(socket).expect("connect to river");
        let conn = Connection::from_socket(stream).expect("wayland connection");
        let (globals, mut queue) = registry_queue_init::<Client>(&conn).expect("registry");
        let qh = queue.handle();
        let seat: WlSeat = globals.bind(&qh, 1..=1, ()).expect("wl_seat");
        let manager: ZwpVirtualKeyboardManagerV1 = globals
            .bind(&qh, 1..=1, ())
            .expect("river supports virtual keyboards");
        let keyboard = manager.create_virtual_keyboard(&seat, &qh, ());
        let pointers: ZwlrVirtualPointerManagerV1 = globals
            .bind(&qh, 1..=2, ())
            .expect("river supports virtual pointers");
        let pointer = pointers.create_virtual_pointer(Some(&seat), &qh, ());

        let groups: Vec<Group<'_>> = Group::from_layouts_and_variants("us", "").collect();
        let keymap = Context::default()
            .keymap_from_names(Vec::<Diagnostic>::new(), None, None, Some(&groups), None)
            .format()
            .to_string();
        let path = session.dir.join("keymap");
        let mut file = File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)
            .expect("keymap file");
        file.write_all(keymap.as_bytes()).expect("write keymap");
        file.write_all(&[0]).expect("write keymap");
        let size = u32::try_from(keymap.len() + 1).expect("keymap size");
        keyboard.keymap(1, file.as_fd(), size);
        queue.roundtrip(&mut Client).expect("roundtrip");
        Self {
            conn,
            keyboard,
            pointer,
            extent,
            time: 0,
        }
    }

    fn tick(&mut self) -> u32 {
        self.time += 10;
        self.time
    }

    fn flush(&self) {
        self.conn.flush().expect("flush");
        sleep(Duration::from_millis(30));
    }

    fn key(&mut self, code: u32, pressed: bool) {
        let time = self.tick();
        self.keyboard.key(time, code, u32::from(pressed));
        self.flush();
    }

    fn tap(&mut self, code: u32) {
        self.key(code, true);
        self.key(code, false);
    }

    /// Moves the pointer to global `(x, y)`.
    fn move_to(&mut self, x: i64, y: i64) {
        let time = self.tick();
        let (w, h) = self.extent;
        let x = u32::try_from(x).expect("x on the output");
        let y = u32::try_from(y).expect("y on the output");
        self.pointer.motion_absolute(time, x, y, w, h);
        self.pointer.frame();
        self.flush();
    }

    fn button(&mut self, pressed: bool) {
        let time = self.tick();
        let state = if pressed {
            wayland_client::protocol::wl_pointer::ButtonState::Pressed
        } else {
            wayland_client::protocol::wl_pointer::ButtonState::Released
        };
        self.pointer.button(time, key::BTN_LEFT, state);
        self.pointer.frame();
        self.flush();
    }

    fn click_at(&mut self, x: i64, y: i64) {
        self.move_to(x, y);
        self.button(true);
        self.button(false);
    }

    fn hold_super(&mut self, held: bool) {
        self.key(key::LEFT_META, held);
        self.keyboard
            .modifiers(if held { MOD4 } else { 0 }, 0, 0, 0);
        self.flush();
    }

    /// Presses `code` while holding Super.
    fn super_tap(&mut self, code: u32) {
        self.hold_super(true);
        self.tap(code);
        self.hold_super(false);
    }

    /// Presses `code` while holding Super and Shift.
    fn super_shift_tap(&mut self, code: u32) {
        self.key(key::LEFT_META, true);
        self.key(key::LEFT_SHIFT, true);
        self.keyboard.modifiers(MOD4 | SHIFT, 0, 0, 0);
        self.tap(code);
        self.key(key::LEFT_SHIFT, false);
        self.key(key::LEFT_META, false);
        self.keyboard.modifiers(0, 0, 0, 0);
        self.flush();
    }
}

/// Opens three terminals and checks they tile below the bar.
fn tiles_three_terminals(session: &mut Session) {
    let state = session.state();
    let screen = &state["screens"][0];
    let area = rect(&screen["rect"]);
    assert_eq!(rect(&state["bar"]["rect"]), [area[0], area[1], area[2], 24]);
    assert_eq!(state["bar"]["name"], screen["name"]);
    assert_eq!(state["current_workspace"], "1");

    for title in ["one", "two", "three"] {
        session.spawn_terminal(title);
        let count = session.state()["managed"].as_u64().unwrap_or(0) + 1;
        wait_for(title, || {
            let state = session.state();
            state["managed"].as_u64() == Some(count)
                && windows(&state).iter().all(|w| !w["size"].is_null())
        });
    }
    let state = session.state();
    let placed = windows(&state);
    assert_eq!(placed.len(), 3, "{state}");
    for (i, a) in placed.iter().enumerate() {
        let a_rect = rect(&a["rect"]);
        assert_eq!(a["layer"], "tiled");
        assert!(a_rect[1] >= 24, "{a} is under the bar");
        for b in &placed[i + 1..] {
            assert!(!overlap(a_rect, rect(&b["rect"])), "{a} overlaps {b}");
        }
    }
    let newest = placed
        .iter()
        .find(|w| w["title"] == "three")
        .expect("three");
    assert_eq!(state["focus"], newest["id"]);
}

/// Full shows one window, tabbed adds a tab strip whose tabs focus on
/// click.
fn cycles_layouts(session: &Session, input: &mut Input) {
    for _ in 0..3 {
        session.act("next-layout");
    }
    let state = session.state();
    assert_eq!(state["screens"][0]["layout"], "Full");
    assert_eq!(windows(&state).len(), 1);
    assert_eq!(state["hidden"].as_array().map(Vec::len), Some(2));

    session.act("next-layout");
    let state = session.state();
    let tabs = &state["screens"][0]["tabs"];
    let titles = tabs["titles"].as_array().cloned().unwrap_or_default();
    assert_eq!(titles.len(), 3, "{state}");
    // Click the last tab: its window gets the focus.
    let strip = rect(&tabs["rect"]);
    input.click_at(strip[0] + strip[2] * 5 / 6, strip[1] + strip[3] / 2);
    wait_for("a tab click to focus its window", || {
        let state = session.state();
        windows(&state).first().map(|w| w["title"].clone()) == titles.last().cloned()
    });
    session.act("reset-layout");
}

fn switches_workspaces(session: &Session, input: &mut Input) {
    session.act("focus-master");
    let master = session.state()["focus"].clone();
    session.act("shift 2");
    assert_eq!(windows(&session.state()).len(), 2);
    session.act("view 2");
    let state = session.state();
    assert_eq!(state["current_workspace"], "2");
    assert_eq!(windows(&state).len(), 1);
    assert_eq!(windows(&state)[0]["id"], master);

    // Clicking workspace 1 in the bar views it.
    let target = state["bar_clicks"]
        .as_array()
        .and_then(|targets| targets.iter().find(|t| t["workspace"] == "1"))
        .cloned()
        .expect("workspace 1 in the bar");
    let span = &target["x"];
    let x = i64::midpoint(span[0].as_i64().unwrap_or(0), span[1].as_i64().unwrap_or(0));
    input.click_at(x, 12);
    wait_for("a bar click to view workspace 1", || {
        session.state()["current_workspace"] == "1"
    });
}

fn toggles_fullscreen(session: &Session) {
    let area = rect(&session.state()["screens"][0]["rect"]);
    session.act("toggle-fullscreen");
    let full: Vec<Value> = windows(&session.state())
        .into_iter()
        .filter(|w| w["layer"] == "fullscreen")
        .collect();
    assert_eq!(full.len(), 1);
    assert_eq!(rect(&full[0]["rect"]), area);
    session.act("toggle-fullscreen");
}

/// `Mod+j`, `Mod+v`, typing into the prompt and Escape.
fn handles_keys_and_prompt(session: &Session, input: &mut Input) {
    let focus = session.state()["focus"].clone();
    input.super_tap(key::J);
    wait_for("Mod+j to move the focus", || {
        session.state()["focus"] != focus
    });

    input.super_tap(key::V);
    wait_for("Mod+v to open the prompt", || {
        session.state()["focus"] == "prompt"
    });
    input.tap(key::THREE);
    wait_for("the prompt to show the input", || {
        session.state()["prompt"]["input"] == "3"
    });
    input.tap(key::ENTER);
    wait_for("the prompt to switch workspace", || {
        let state = session.state();
        state["current_workspace"] == "3" && state["prompt"].is_null()
    });

    input.super_tap(key::V);
    wait_for("the prompt to reopen", || {
        session.state()["focus"] == "prompt"
    });
    // A held key repeats.
    for _ in 0..4 {
        input.tap(key::THREE);
    }
    wait_for("typing", || session.state()["prompt"]["input"] == "3333");
    input.key(key::BACKSPACE, true);
    wait_for("a held BackSpace to repeat", || {
        session.state()["prompt"]["input"] == ""
    });
    input.key(key::BACKSPACE, false);
    input.tap(key::ESC);
    wait_for("Escape to close the prompt", || {
        session.state()["prompt"].is_null()
    });
    session.act("view 1");
}

/// `Mod+drag` floats the window under the pointer and moves it.
fn drags_windows(session: &Session, input: &mut Input) {
    let state = session.state();
    let window = windows(&state).last().cloned().expect("a window");
    let start = rect(&window["rect"]);
    let (x, y) = (start[0] + start[2] / 2, start[1] + start[3] / 2);
    input.move_to(x, y);
    input.hold_super(true);
    input.button(true);
    input.move_to(x + 40, y + 30);
    input.button(false);
    input.hold_super(false);
    wait_for("the dragged window to float at its new place", || {
        let state = session.state();
        windows(&state).iter().any(|w| {
            let r = rect(&w["rect"]);
            w["id"] == window["id"]
                && w["layer"] == "floating"
                && (r[0], r[1]) == (start[0] + 40, start[1] + 30)
        })
    });
    session.act("sink");
}

#[test]
#[ignore = "needs river and alacritty; run with --ignored"]
fn manages_windows_under_headless_river() {
    if !have("river") || !have("alacritty") {
        eprintln!("skipping: river or alacritty is not installed");
        return;
    }
    let mut session = Session::start();
    tiles_three_terminals(&mut session);
    let area = rect(&session.state()["screens"][0]["rect"]);
    let extent = (
        u32::try_from(area[2]).expect("width"),
        u32::try_from(area[3]).expect("height"),
    );
    let mut input = Input::connect(&session, extent);

    cycles_layouts(&session, &mut input);
    switches_workspaces(&session, &mut input);
    toggles_fullscreen(&session);
    handles_keys_and_prompt(&session, &mut input);
    drags_windows(&session, &mut input);

    session.act("close");
    wait_for("the window to close", || {
        session.state()["managed"].as_u64() == Some(2)
    });

    session.act("quit");
    wait_for("river to exit", || {
        session.river.try_wait().ok().flatten().is_some()
    });
    let log = session.log();
    assert!(!log.contains("protocol error"), "{log}");
    assert!(!log.contains("ERROR"), "{log}");
}

// --------------------------------------------------------------------
// Restart in place

/// The children of process `pid`.
fn children_of(pid: u32) -> Vec<u32> {
    fs::read_to_string(format!("/proc/{pid}/task/{pid}/children"))
        .unwrap_or_default()
        .split_whitespace()
        .filter_map(|p| p.parse().ok())
        .collect()
}

/// oxmonad's pid: river's only child.
fn wm_pid(session: &Session) -> u32 {
    let children = children_of(session.river.id());
    assert_eq!(children.len(), 1, "river's children: {children:?}");
    children[0]
}

/// oxmonad's open file descriptors by kind (`socket`, `pipe`,
/// `anon_inode:[eventpoll]`, a path…), leaving out the memfd-backed shell
/// buffers (their number depends on the redraw history). Waits until the
/// set is stable, so control connections being closed do not count.
fn open_fds(pid: u32) -> Vec<String> {
    let read = || {
        let mut fds: Vec<String> = fs::read_dir(format!("/proc/{pid}/fd"))
            .map(|dir| {
                dir.filter_map(Result::ok)
                    .filter_map(|e| fs::read_link(e.path()).ok())
                    .map(|target| {
                        let target = target.to_string_lossy().into_owned();
                        target.split(":[").next().unwrap_or_default().to_owned()
                    })
                    .filter(|target| !target.starts_with("/memfd:"))
                    .collect()
            })
            .unwrap_or_default();
        fds.sort();
        fds
    };
    let mut last = read();
    loop {
        sleep(Duration::from_millis(200));
        let now = read();
        if now == last {
            return now;
        }
        last = now;
    }
}

/// What must survive a restart: the arrangement with windows named by
/// river's identifiers instead of oxmonad's ids.
fn arrangement(state: &Value) -> Value {
    let placed = windows(state);
    let name = |id: &Value| {
        placed
            .iter()
            .find(|w| w["id"] == *id)
            .map_or(Value::Null, |w| w["identifier"].clone())
    };
    let screens: Vec<Value> = state["screens"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|s| {
            serde_json::json!({
                "name": s["name"],
                "workspace": s["workspace"],
                "layout": s["layout"],
                "rect": s["rect"],
                "usable": s["usable"],
                "split": s["split"],
                "tabs": s["tabs"],
            })
        })
        .collect();
    let placed_now: Vec<Value> = placed
        .iter()
        .map(|w| {
            serde_json::json!({
                "identifier": w["identifier"],
                "title": w["title"],
                "app_id": w["app_id"],
                "layer": w["layer"],
                "rect": w["rect"],
                "border": w["border"],
            })
        })
        .collect();
    serde_json::json!({
        "current_workspace": state["current_workspace"],
        "screens": screens,
        "bar": state["bar"]["rect"],
        "windows": placed_now,
        "hidden": state["hidden"].as_array().map(Vec::len),
        "focus": name(&state["focus"]),
        "bar_visible": state["bar_visible"],
        "managed": state["managed"],
    })
}

/// The arrangement once every window has its size, or `None` if oxmonad
/// does not answer.
///
/// A window that was fullscreen before a restart has no size afterwards:
/// river keeps it fullscreen and does not report its dimensions to the new
/// window manager again.
fn settled(session: &Session) -> Option<Value> {
    let state = session.try_state()?;
    windows(&state)
        .iter()
        .all(|w| (!w["size"].is_null() || w["layer"] == "fullscreen") && !w["identifier"].is_null())
        .then(|| arrangement(&state))
}

/// The arrangement of workspaces 1, 2 and 3 (ending on 1).
fn observe(session: &Session) -> Vec<Value> {
    let mut seen = Vec::new();
    for tag in ["1", "2", "3"] {
        session.act(&format!("view {tag}"));
        let deadline = Instant::now() + Duration::from_secs(15);
        let arrangement = loop {
            if let Some(arrangement) = settled(session) {
                break arrangement;
            }
            assert!(
                Instant::now() < deadline,
                "workspace {tag} does not settle: {}",
                session.command("dump-state")
            );
            sleep(Duration::from_millis(50));
        };
        seen.push(arrangement);
    }
    session.act("view 1");
    seen
}

/// Sends `restart` and waits until oxmonad shows `expected` again.
fn restart(session: &Session, expected: &Value) {
    session.act("restart");
    let mut last = None;
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        last = settled(session);
        if last.as_ref() == Some(expected) {
            return;
        }
        sleep(Duration::from_millis(100));
    }
    assert_eq!(last.as_ref(), Some(expected), "{}", session.log());
}

/// Replaces the binary like cargo does: a new file renamed over the old,
/// whose inode is then gone.
fn rebuild(binary: &Path) {
    let fresh = binary.with_extension("new");
    fs::copy(env!("CARGO_BIN_EXE_oxmonad"), &fresh).expect("copy the binary");
    fs::rename(&fresh, binary).expect("replace the binary");
}

fn spawn_and_wait(session: &mut Session, title: &str) {
    let count = session.state()["managed"].as_u64().unwrap_or(0) + 1;
    session.spawn_terminal(title);
    wait_for(title, || session.state()["managed"].as_u64() == Some(count));
}

/// The content rectangle of a window floated by a rule on the 1280x720
/// screen: the middle quarter, less the 1px border.
const RULE_FLOAT: [i64; 4] = [321, 181, 638, 358];

/// The window floated by the `floater` rule.
fn rule_float(session: &Session) -> Value {
    let state = session.state();
    let window = windows(&state)
        .into_iter()
        .find(|w| w["app_id"] == "floater")
        .unwrap_or_else(|| panic!("the rule-floated window is shown: {state}"));
    assert_eq!(window["layer"], "floating", "{state}");
    window
}

/// A non-trivial state: windows on three workspaces (one copied, one
/// fullscreen), a floating window, a changed layout with two masters and a
/// wider master column, focus off the master, a hidden scratchpad and the
/// bar hidden.
fn arrange_for_restart(session: &mut Session, input: &mut Input) {
    for title in ["one", "two", "three"] {
        spawn_and_wait(session, title);
    }
    session.act("shift 2");
    session.act("view 2");
    session.act("toggle-fullscreen");
    session.act("view 1");
    for title in ["four", "five"] {
        spawn_and_wait(session, title);
    }
    // Float "five" by dragging it.
    let five = windows(&session.state())
        .into_iter()
        .find(|w| w["title"] == "five")
        .expect("five");
    let start = rect(&five["rect"]);
    let (x, y) = (start[0] + start[2] / 2, start[1] + start[3] / 2);
    input.move_to(x, y);
    input.hold_super(true);
    input.button(true);
    input.move_to(x - 50, y + 30);
    input.button(false);
    input.hold_super(false);
    wait_for("five to float", || {
        windows(&session.state())
            .iter()
            .any(|w| w["id"] == five["id"] && w["layer"] == "floating")
    });
    // "six" floats by a manage rule, centred at half the screen size.
    let count = session.state()["managed"].as_u64().unwrap_or(0) + 1;
    session.spawn_terminal_with("six", &["--class", "floater"]);
    wait_for("six", || session.state()["managed"].as_u64() == Some(count));
    assert_eq!(rule_float(session)["rect"], serde_json::json!(RULE_FLOAT));
    session.act("next-layout");
    session.act("expand");
    session.act("inc-master 1");
    session.act("focus-master");
    session.act("focus-prev");
    session.act("copy 3");

    let count = session.state()["managed"].as_u64().unwrap_or(0) + 1;
    session.act("toggle-scratchpad term");
    wait_for("the scratchpad", || {
        session.state()["managed"].as_u64() == Some(count)
    });
    session.act("toggle-scratchpad term");
    session.act("toggle-bar");

    let state = session.state();
    let placed = windows(&state);
    assert!(placed.iter().any(|w| w["layer"] == "floating"), "{state}");
    let master = placed.iter().find(|w| w["layer"] == "tiled");
    assert_ne!(Some(&state["focus"]), master.map(|w| &w["id"]), "{state}");
    assert_eq!(state["bar_visible"], false);
}

/// `oxmonad --restart` restarts the oxmonad of its own session only.
fn restarts_via_cli(session: &Session, binary: &Path, current: &Value) {
    // `oxmonad --restart`, run inside the session, restarts it too.
    let restores = |s: &Session| s.log().matches("restoring the state").count();
    let before = restores(session);
    let output = Command::new(binary)
        .arg("--restart")
        .env("XDG_RUNTIME_DIR", &session.dir)
        .env("WAYLAND_DISPLAY", session.wayland_display())
        .output()
        .expect("oxmonad --restart runs");
    assert!(output.status.success(), "{output:?}");
    wait_for("the CLI restart", || restores(session) > before);
    wait_for("the arrangement after the CLI restart", || {
        settled(session).as_ref() == Some(current)
    });
    // Outside the session there is nothing to restart.
    let output = Command::new(binary)
        .arg("--restart")
        .env("XDG_RUNTIME_DIR", session.dir.join("elsewhere"))
        .env("WAYLAND_DISPLAY", "wayland-99")
        .output()
        .expect("oxmonad --restart runs");
    assert!(!output.status.success(), "{output:?}");
}

#[test]
#[ignore = "needs river and alacritty; run with --ignored"]
fn restarts_in_place() {
    if !have("river") || !have("alacritty") {
        eprintln!("skipping: river or alacritty is not installed");
        return;
    }
    let dir = Session::dir("restart");
    let config = dir.join("config/oxmonad");
    fs::create_dir_all(&config).expect("config dir");
    fs::write(
        config.join("config.toml"),
        "[[rules]]\napp_id = \"floater\"\naction = \"float\"\n",
    )
    .expect("config");
    let bin = dir.join("bin");
    fs::create_dir_all(&bin).expect("bin dir");
    let binary = bin.join("oxmonad");
    fs::copy(env!("CARGO_BIN_EXE_oxmonad"), &binary).expect("copy the binary");
    let mut session = Session::start_in(dir, &binary);

    let area = rect(&session.state()["screens"][0]["rect"]);
    let extent = (
        u32::try_from(area[2]).expect("width"),
        u32::try_from(area[3]).expect("height"),
    );
    let mut input = Input::connect(&session, extent);
    arrange_for_restart(&mut session, &mut input);
    let before = observe(&session);
    let current = before[0].clone();
    let pid = wm_pid(&session);
    let scratchpad = children_of(pid);
    assert_eq!(scratchpad.len(), 1, "the scratchpad is oxmonad's child");
    let fds = open_fds(pid);
    let unchanged = |session: &mut Session| {
        assert_eq!(observe(session), before);
        assert_eq!(wm_pid(session), pid, "oxmonad keeps its pid");
        assert!(session.river.try_wait().expect("river").is_none());
        for client in &mut session.clients {
            assert!(client.try_wait().expect("alacritty").is_none());
        }
        assert_eq!(children_of(pid), scratchpad, "the scratchpad survives");
        assert_eq!(rule_float(session)["rect"], serde_json::json!(RULE_FLOAT));
    };

    // A rebuilt binary replaces the running one.
    rebuild(&binary);
    restart(&session, &current);
    unchanged(&mut session);
    assert_eq!(open_fds(pid), fds, "no descriptor leaks across exec");
    // The scratchpad is still known: toggling shows the same window.
    session.act("toggle-scratchpad term");
    let state = session.state();
    assert_eq!(state["managed"], current["managed"]);
    assert!(
        windows(&state).iter().any(|w| w["app_id"] == "scratchpad"),
        "{state}"
    );
    session.act("toggle-scratchpad term");
    wait_for("the scratchpad to hide", || {
        settled(&session).as_ref() == Some(&current)
    });

    // Again, from the restarted process.
    fs::remove_file(&binary).expect("remove the binary");
    fs::copy(env!("CARGO_BIN_EXE_oxmonad"), &binary).expect("copy the binary");
    restart(&session, &current);
    unchanged(&mut session);
    assert_eq!(open_fds(pid), fds, "no descriptor leaks across exec");

    // Without a binary, oxmonad refuses to restart and keeps running.
    fs::remove_file(&binary).expect("remove the binary");
    restart(&session, &current);
    unchanged(&mut session);
    assert!(session.log().contains("keeping this oxmonad"));

    // If exec fails after the check, oxmonad resumes in-process.
    fs::write(&binary, "#!/bin/sh\nrm -f \"$0\"\n").expect("write the script");
    let mut permissions = fs::metadata(&binary).expect("script").permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut permissions, 0o755);
    fs::set_permissions(&binary, permissions).expect("make it executable");
    restart(&session, &current);
    unchanged(&mut session);
    assert!(session.log().contains("window management resumed"));

    // And restarting works again afterwards.
    fs::copy(env!("CARGO_BIN_EXE_oxmonad"), &binary).expect("copy the binary");
    restart(&session, &current);
    unchanged(&mut session);

    restarts_via_cli(&session, &binary, &current);
    unchanged(&mut session);

    for child in scratchpad {
        let _ = Command::new("kill")
            .args(["-TERM", "--", &format!("-{child}")])
            .status();
    }
    session.act("quit");
    wait_for("river to exit", || {
        session.river.try_wait().ok().flatten().is_some()
    });
    let log = session.log();
    assert!(!log.contains("protocol error"), "{log}");
    assert!(!log.contains("panicked"), "{log}");
}

// --------------------------------------------------------------------
// Virtual screens

/// The windows titled `title` in `state`.
fn window_titled(state: &Value, title: &str) -> Value {
    windows(state)
        .into_iter()
        .find(|w| w["title"] == title)
        .unwrap_or_else(|| panic!("{title} is shown: {state}"))
}

/// Whether `inner` lies within `outer`.
fn within(inner: [i64; 4], outer: [i64; 4]) -> bool {
    inner[0] >= outer[0]
        && inner[1] >= outer[1]
        && inner[0] + inner[2] <= outer[0] + outer[2]
        && inner[1] + inner[3] <= outer[1] + outer[3]
}

/// Saves a screenshot to `$OXMONAD_E2E_SCREENSHOTS/<name>.png` if that
/// variable is set (and `grim` is installed).
fn screenshot(session: &Session, name: &str) {
    let Some(dir) = env::var_os("OXMONAD_E2E_SCREENSHOTS") else {
        return;
    };
    let path = PathBuf::from(dir).join(format!("{name}.png"));
    let status = Command::new("grim")
        .arg(&path)
        .env("XDG_RUNTIME_DIR", &session.dir)
        .env("WAYLAND_DISPLAY", session.wayland_display())
        .status()
        .expect("grim runs");
    assert!(status.success(), "grim {}", path.display());
}

/// `split-screen 2`: two tilings side by side under one bar spanning the
/// output, a fullscreen window covering only its half, a restart keeping
/// the split, and `rescreen` merging back without losing a window.
#[test]
#[ignore = "needs river and alacritty; run with --ignored"]
fn splits_screens() {
    if !have("river") || !have("alacritty") {
        eprintln!("skipping: river or alacritty is not installed");
        return;
    }
    let mut session = Session::start_in(
        Session::dir("split"),
        Path::new(env!("CARGO_BIN_EXE_oxmonad")),
    );
    let output = rect(&session.state()["screens"][0]["rect"]);
    let [x, y, w, h] = output;
    spawn_and_wait(&mut session, "left");

    session.act("split-screen 2");
    let state = session.state();
    let screens = state["screens"].as_array().cloned().unwrap_or_default();
    assert_eq!(screens.len(), 2, "{state}");
    let (left, right) = (rect(&screens[0]["rect"]), rect(&screens[1]["rect"]));
    assert_eq!(left, [x, y, w / 2, h]);
    assert_eq!(right, [x + w / 2, y, w - w / 2, h]);
    assert_eq!(screens[0]["output"], screens[1]["output"]);
    assert_eq!(screens[0]["split"], true);
    assert_eq!(
        (&screens[0]["workspace"], &screens[1]["workspace"]),
        (&Value::from("1"), &Value::from("2"))
    );
    // One bar across the whole output.
    assert_eq!(rect(&state["bar"]["rect"]), [x, y, w, 24]);
    assert_eq!(
        rect(&window_titled(&state, "left")["rect"]),
        [x + 1, y + 25, w / 2 - 2, h - 26]
    );

    session.act("focus-screen next");
    for title in ["right one", "right two"] {
        spawn_and_wait(&mut session, title);
    }
    wait_for("every window to get its size", || {
        windows(&session.state())
            .iter()
            .all(|w| !w["size"].is_null())
    });
    let state = session.state();
    assert_eq!(state["current_workspace"], "2");
    let left_area = [left[0], left[1] + 24, left[2], left[3] - 24];
    let right_area = [right[0], right[1] + 24, right[2], right[3] - 24];
    assert!(within(
        rect(&window_titled(&state, "left")["rect"]),
        left_area
    ));
    for title in ["right one", "right two"] {
        let window = window_titled(&state, title);
        assert!(within(rect(&window["rect"]), right_area), "{window}");
        assert_eq!(window["layer"], "tiled");
    }
    screenshot(&session, "split");

    // Fullscreen covers the right half only, over the bar.
    session.act("toggle-fullscreen");
    wait_for("the fullscreen window to fill its half", || {
        let window = window_titled(&session.state(), "right two");
        window["layer"] == "virtualfullscreen"
            && rect(&window["rect"]) == right
            && window["size"] == serde_json::json!([right[2], right[3]])
            && window["position"] == serde_json::json!([right[0], right[1]])
    });
    let state = session.state();
    assert_eq!(window_titled(&state, "left")["layer"], "tiled");
    screenshot(&session, "split-fullscreen");

    // A restart keeps the split, the workspaces and the fullscreen window.
    let before = settled(&session).expect("settled");
    let restores = |s: &Session| s.log().matches("restoring the state").count();
    restart(&session, &before);
    wait_for("the restarted oxmonad", || restores(&session) == 1);
    restart(&session, &before);

    // rescreen: one screen again, the current workspace keeps the output,
    // the left half's workspace is hidden, no window is lost.
    session.act("rescreen");
    let state = session.state();
    let screens = state["screens"].as_array().cloned().unwrap_or_default();
    assert_eq!(screens.len(), 1, "{state}");
    assert_eq!(rect(&screens[0]["rect"]), output);
    assert_eq!(screens[0]["split"], false);
    assert_eq!(state["current_workspace"], "2");
    assert_eq!(state["managed"], 3);
    let full = window_titled(&state, "right two");
    assert_eq!(full["layer"], "fullscreen");
    assert_eq!(rect(&full["rect"]), output);
    assert!(windows(&state).iter().all(|w| w["title"] != "left"));
    session.act("toggle-ws");
    wait_for("the left half's workspace to come back", || {
        let state = session.state();
        state["current_workspace"] == "1"
            && windows(&state).iter().any(|window| {
                window["title"] == "left" && within(rect(&window["rect"]), [x, y + 24, w, h - 24])
            })
    });

    session.act("quit");
    wait_for("river to exit", || {
        session.river.try_wait().ok().flatten().is_some()
    });
    let log = session.log();
    assert!(!log.contains("protocol error"), "{log}");
    assert!(!log.contains("panicked"), "{log}");
}

/// The layout picker: `pick-layout` opens it with the active layout
/// selected, `j`/`k` move the selection, `Return` applies it, a click on a
/// card applies that layout, `Escape` and `pick-layout` close it unchanged.
#[test]
#[ignore = "needs river and alacritty; run with --ignored"]
fn picks_layouts() {
    if !have("river") || !have("alacritty") {
        eprintln!("skipping: river or alacritty is not installed");
        return;
    }
    let mut session = Session::start_in(
        Session::dir("picker"),
        Path::new(env!("CARGO_BIN_EXE_oxmonad")),
    );
    for title in ["one", "two", "three"] {
        spawn_and_wait(&mut session, title);
    }
    let area = rect(&session.state()["screens"][0]["rect"]);
    let extent = (
        u32::try_from(area[2]).expect("width"),
        u32::try_from(area[3]).expect("height"),
    );
    let mut input = Input::connect(&session, extent);
    let layout = |s: &Session| s.state()["screens"][0]["layout"].clone();
    let open = |s: &Session| {
        s.act("pick-layout");
        wait_for("the picker to open with its cards", || {
            let state = s.state();
            state["focus"] == "layout-picker"
                && state["layout_picker"]["cards"]
                    .as_array()
                    .is_some_and(|cards| cards.len() == 10)
        });
        s.state()["layout_picker"].clone()
    };

    let picker = open(&session);
    assert_eq!(picker["selected"], 0, "{picker}");
    assert_eq!(picker["active"], 0);
    assert_eq!(picker["entries"][0], "Tall Right");
    // Centred on the screen, above everything.
    let shell = session.state()["shells"]
        .as_array()
        .and_then(|shells| shells.iter().find(|s| s["node"] == "LayoutPicker").cloned())
        .expect("the picker surface");
    let r = rect(&shell["rect"]);
    assert!(within(r, area), "{shell}");
    assert!((r[0] - area[0] - (area[0] + area[2] - r[0] - r[2])).abs() <= 1);
    screenshot(&session, "layout-picker");

    for code in [key::J, key::J, key::K] {
        input.tap(code);
    }
    wait_for("j/j/k to select the second layout", || {
        session.state()["layout_picker"]["selected"] == 1
    });
    assert_eq!(layout(&session), "Tall Right");
    input.tap(key::ENTER);
    wait_for("Return to apply Tall", || {
        let state = session.state();
        state["layout_picker"].is_null() && state["screens"][0]["layout"] == "Tall"
    });
    assert_ne!(session.state()["focus"], "layout-picker");

    // A click on a card applies its layout.
    let picker = open(&session);
    assert_eq!(picker["active"], 1);
    let grid = picker["entries"]
        .as_array()
        .and_then(|names| names.iter().position(|n| n == "Grid"))
        .expect("Grid is offered");
    let card = rect(&picker["cards"][grid]);
    // Hovering highlights the card under the pointer without applying it.
    input.move_to(card[0] + card[2] / 2, card[1] + card[3] / 2);
    wait_for("the hovered card to be selected", || {
        session.state()["layout_picker"]["selected"] == grid
    });
    assert_eq!(layout(&session), "Tall");
    input.click_at(card[0] + card[2] / 2, card[1] + card[3] / 2);
    wait_for("the click to apply Grid", || {
        let state = session.state();
        state["layout_picker"].is_null() && state["screens"][0]["layout"] == "Grid"
    });

    // Escape and a second pick-layout close it without a change.
    open(&session);
    input.tap(key::J);
    wait_for("j to move the selection", || {
        session.state()["layout_picker"]["selected"] != 6
    });
    input.tap(key::ESC);
    wait_for("Escape to close the picker", || {
        session.state()["layout_picker"].is_null()
    });
    open(&session);
    session.act("pick-layout");
    assert!(session.state()["layout_picker"].is_null());
    assert_eq!(layout(&session), "Grid");
    assert_ne!(session.state()["focus"], "layout-picker");

    session.act("quit");
    wait_for("river to exit", || {
        session.river.try_wait().ok().flatten().is_some()
    });
    let log = session.log();
    assert!(!log.contains("protocol error"), "{log}");
    assert!(!log.contains("panicked"), "{log}");
}

/// The key binding help: `Super+Shift+/` and the `help` action open it with
/// the keyboard focus, `Escape`, `q`, a click and a second `help` close it.
#[test]
#[ignore = "needs river and alacritty; run with --ignored"]
fn shows_key_help() {
    if !have("river") || !have("alacritty") {
        eprintln!("skipping: river or alacritty is not installed");
        return;
    }
    let mut session = Session::start_in(
        Session::dir("help"),
        Path::new(env!("CARGO_BIN_EXE_oxmonad")),
    );
    spawn_and_wait(&mut session, "one");
    let area = rect(&session.state()["screens"][0]["rect"]);
    let extent = (
        u32::try_from(area[2]).expect("width"),
        u32::try_from(area[3]).expect("height"),
    );
    let mut input = Input::connect(&session, extent);
    let window = session.state()["focus"].clone();
    assert!(window.is_u64(), "a window has focus: {window}");
    let shell = |s: &Session| {
        s.state()["shells"]
            .as_array()
            .and_then(|shells| shells.iter().find(|s| s["node"] == "Help").cloned())
    };
    let wait_open = |s: &Session, how: &str| {
        wait_for(how, || {
            let state = s.state();
            state["focus"] == "help" && !state["help"].is_null()
        });
        wait_for("the help surface", || shell(s).is_some());
        s.state()["help"].clone()
    };
    let wait_closed = |s: &Session, how: &str| {
        wait_for(how, || {
            let state = s.state();
            state["help"].is_null() && state["focus"] == window
        });
        assert!(shell(s).is_none());
    };

    // The default binding, typed.
    input.super_shift_tap(key::SLASH);
    let help = wait_open(&session, "Super+Shift+slash to open the help");
    assert_eq!(help["scroll"], 0, "{help}");
    assert_eq!(
        help["sections"],
        serde_json::json!([
            "Launch",
            "Windows",
            "Layouts",
            "Workspaces",
            "Screens",
            "Session",
            "Mouse"
        ])
    );
    assert!(help["bindings"].as_u64().is_some_and(|n| n >= 50), "{help}");
    // Centred on the screen, above everything.
    let r = rect(&shell(&session).expect("the help surface")["rect"]);
    assert!(within(r, area), "{r:?}");
    assert!((r[0] - area[0] - (area[0] + area[2] - r[0] - r[2])).abs() <= 1);
    screenshot(&session, "help");
    // Not everything fits the 1280x720 output: j/k and PageDown scroll,
    // stopping at the end.
    let max_scroll = help["max_scroll"].as_u64().expect("max_scroll");
    assert!(max_scroll > 0, "{help}");
    let scroll = |s: &Session| s.state()["help"]["scroll"].as_u64();
    input.tap(key::J);
    wait_for("j to scroll a row", || scroll(&session) == Some(1));
    input.tap(key::K);
    wait_for("k to scroll back", || scroll(&session) == Some(0));
    for _ in 0..3 {
        input.tap(key::PAGE_DOWN);
    }
    wait_for("PageDown to scroll to the end", || {
        scroll(&session) == Some(max_scroll)
    });
    screenshot(&session, "help-scrolled");
    input.tap(key::ESC);
    wait_closed(&session, "Escape to close the help");

    session.act("help");
    wait_open(&session, "the help action to open it");
    input.tap(key::Q);
    wait_closed(&session, "q to close the help");

    session.act("help");
    wait_open(&session, "the help action to open it again");
    let r = rect(&shell(&session).expect("the help surface")["rect"]);
    input.click_at(r[0] + r[2] / 2, r[1] + r[3] / 2);
    wait_closed(&session, "a click to close the help");

    // Exclusive with the layout picker; a second `help` closes it.
    session.act("pick-layout");
    session.act("help");
    wait_open(&session, "help to replace the layout picker");
    assert!(session.state()["layout_picker"].is_null());
    session.act("help");
    wait_closed(&session, "a second help to close it");

    session.act("quit");
    wait_for("river to exit", || {
        session.river.try_wait().ok().flatten().is_some()
    });
    let log = session.log();
    assert!(!log.contains("protocol error"), "{log}");
    assert!(!log.contains("panicked"), "{log}");
}
