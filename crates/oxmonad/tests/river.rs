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
    path::PathBuf,
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
    pub const V: u32 = 47;
    pub const ENTER: u32 = 28;
    pub const BACKSPACE: u32 = 14;
    pub const LEFT_META: u32 = 125;
    pub const BTN_LEFT: u32 = 0x110;
}

/// The xkb modifier mask of Mod4 in the `us` keymap.
const MOD4: u32 = 1 << 6;

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
    fn start() -> Self {
        let dir = env::temp_dir().join(format!("oxm-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("config")).expect("runtime dir");
        let log = File::create(dir.join("river.log")).expect("log file");
        let river = Command::new("river")
            .args(["-no-xwayland", "-c", env!("CARGO_BIN_EXE_oxmonad")])
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
        let mut stream = UnixStream::connect(self.control_path()).expect("control socket");
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("timeout");
        writeln!(stream, "{line}").expect("send");
        let mut answer = String::new();
        BufReader::new(stream)
            .read_line(&mut answer)
            .expect("answer");
        answer.trim().to_owned()
    }

    fn act(&self, action: &str) {
        assert_eq!(self.command(action), "ok", "{action}");
    }

    fn state(&self) -> Value {
        serde_json::from_str(&self.command("dump-state")).expect("dump-state is JSON")
    }

    fn spawn_terminal(&mut self, title: &str) {
        let child = Command::new("alacritty")
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
}

/// Opens three terminals and checks they tile below the bar.
fn tiles_three_terminals(session: &mut Session) {
    let state = session.state();
    let screen = &state["screens"][0];
    let area = rect(&screen["rect"]);
    assert_eq!(rect(&screen["bar"]), [area[0], area[1], area[2], 24]);
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
