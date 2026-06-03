use std::{
    io::{
        BufRead,
        BufReader,
        Read,
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
    sync::mpsc,
    time::{
        Duration,
        Instant,
        SystemTime,
        UNIX_EPOCH,
    },
};

const TOLERANCE: u8 = 24;
const MAX_DIFFERING_PIXELS: usize = 4_000;

#[test]
#[ignore = "launches real Alacritty clients against the headless compositor backend"]
fn snapshot_single_window() {
    assert_real_terminal_snapshot("single_window", 1, &[]);
}

#[test]
#[ignore = "launches real Alacritty clients against the headless compositor backend"]
fn snapshot_two_windows_master_focused() {
    assert_real_terminal_snapshot("two_windows_master_focused", 2, &[]);
}

#[test]
#[ignore = "launches real Alacritty clients against the headless compositor backend"]
fn snapshot_three_windows_stack_focused() {
    assert_real_terminal_snapshot("three_windows_stack_focused", 3, &["focus-next"]);
}

fn assert_real_terminal_snapshot(name: &str, terminal_count: usize, actions: &[&str]) {
    require_command("alacritty");

    let paths = CapturePaths::new(name);
    let mut oxmonad = spawn_oxmonad(&paths.raw, terminal_count, actions);
    let display = wait_for_wayland_display(&mut oxmonad);
    let mut terminals = spawn_terminals(&display, terminal_count);

    wait_for_exit(&mut oxmonad, Duration::from_secs(12));
    cleanup_children(&mut terminals);

    let frame = read_capture(&paths.raw);
    let actual = frame.to_rgba_image();

    if std::env::var_os("UPDATE_REAL_TERMINAL_SNAPSHOTS").is_some() || !paths.golden.exists() {
        actual.save(&paths.golden).expect("write golden image");
        assert!(
            std::env::var_os("UPDATE_REAL_TERMINAL_SNAPSHOTS").is_some(),
            "created {}; rerun the test to verify it",
            paths.golden.display(),
        );
        return;
    }

    let expected = image::open(&paths.golden)
        .expect("open golden image")
        .to_rgba8();
    assert_eq!(
        (expected.width(), expected.height()),
        (actual.width(), actual.height()),
        "{name}: real terminal snapshot dimensions differ from golden",
    );

    let differing = expected
        .pixels()
        .zip(actual.pixels())
        .filter(|(expected, actual)| {
            expected
                .0
                .iter()
                .zip(actual.0.iter())
                .any(|(x, y)| x.abs_diff(*y) > TOLERANCE)
        })
        .count();

    if differing > MAX_DIFFERING_PIXELS {
        actual.save(&paths.actual).expect("write actual image");
        panic!(
            "{name}: real terminal snapshot differs in {differing} pixels; wrote {}",
            paths.actual.display()
        );
    }
}

#[derive(Debug)]
struct CapturePaths {
    raw: PathBuf,
    golden: PathBuf,
    actual: PathBuf,
}

impl CapturePaths {
    fn new(name: &str) -> Self {
        let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        let snapshot_dir = manifest_dir.join("tests/snapshots");
        std::fs::create_dir_all(&snapshot_dir).expect("create snapshot directory");

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time is after epoch")
            .as_nanos();

        Self {
            raw: std::env::temp_dir().join(format!("{name}-{unique}.argb")),
            golden: snapshot_dir.join(format!("{name}.png")),
            actual: snapshot_dir.join(format!("{name}.actual.png")),
        }
    }
}

#[derive(Debug)]
struct CapturedFrame {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

impl CapturedFrame {
    fn to_rgba_image(&self) -> image::RgbaImage {
        let mut rgba = Vec::with_capacity(self.pixels.len());
        for pixel in self.pixels.chunks_exact(4) {
            rgba.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
        }

        image::RgbaImage::from_raw(self.width, self.height, rgba).expect("build image from pixels")
    }
}

fn require_command(command: &str) {
    let status = Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {command} >/dev/null 2>&1"))
        .status()
        .expect("check command availability");
    assert!(
        status.success(),
        "{command} is required for this ignored test"
    );
}

fn spawn_oxmonad(raw_capture: &Path, expected_windows: usize, actions: &[&str]) -> Child {
    let mut command = Command::new(env!("CARGO_BIN_EXE_oxmonad"));
    command
        .env("RUST_LOG", "warn")
        .env("OXMONAD_BACKEND", "headless")
        .env("OXMONAD_CAPTURE_FRAME", raw_capture)
        .env("OXMONAD_CAPTURE_AFTER_FRAMES", "90")
        .env(
            "OXMONAD_HEADLESS_EXPECT_WINDOWS",
            expected_windows.to_string(),
        )
        .stdout(Stdio::null())
        .stderr(Stdio::piped());

    if !actions.is_empty() {
        command.env("OXMONAD_HEADLESS_ACTIONS", actions.join(","));
    }

    command.spawn().expect("spawn oxmonad")
}

fn wait_for_wayland_display(oxmonad: &mut Child) -> String {
    let stderr = oxmonad.stderr.take().expect("oxmonad stderr is piped");
    let (tx, rx) = mpsc::channel();

    std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines() {
            let Ok(line) = line else {
                break;
            };
            if let Some(display) = line.strip_prefix("OXMONAD_WAYLAND_DISPLAY=") {
                let _ = tx.send(display.to_string());
            }
        }
    });

    rx.recv_timeout(Duration::from_secs(5))
        .expect("timed out waiting for oxmonad Wayland socket")
}

fn spawn_terminals(display: &str, count: usize) -> Vec<Child> {
    (1..=count)
        .map(|index| {
            Command::new("alacritty")
                .env("WAYLAND_DISPLAY", display)
                .arg("--title")
                .arg(format!("real-terminal-{index}"))
                .arg("-e")
                .arg("sh")
                .arg("-c")
                .arg(format!(
                    "printf 'real terminal {index}\\noxmonad snapshot\\n'; sleep 30"
                ))
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("spawn alacritty")
        })
        .collect()
}

fn wait_for_exit(child: &mut Child, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if child.try_wait().expect("poll child").is_some() {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    let _ = child.kill();
    panic!("oxmonad did not exit after writing the frame capture");
}

fn cleanup_children(children: &mut [Child]) {
    for child in children {
        let _ = child.kill();
        let _ = child.wait();
    }
}

fn read_capture(path: &Path) -> CapturedFrame {
    let mut file = std::fs::File::open(path).expect("open raw capture");
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).expect("read raw capture");

    let newline = bytes
        .iter()
        .position(|byte| *byte == b'\n')
        .expect("capture has header newline");
    let header = std::str::from_utf8(&bytes[..newline]).expect("capture header is utf8");
    let parts = header.split_whitespace().collect::<Vec<_>>();
    assert_eq!(
        parts.first().copied(),
        Some("OXMONAD_ARGB8888"),
        "unexpected capture format"
    );

    let width = parts
        .get(1)
        .expect("capture width")
        .parse::<u32>()
        .expect("parse capture width");
    let height = parts
        .get(2)
        .expect("capture height")
        .parse::<u32>()
        .expect("parse capture height");
    let stride = parts
        .get(3)
        .expect("capture stride")
        .parse::<usize>()
        .expect("parse capture stride");

    assert_eq!(
        stride,
        usize::try_from(width).expect("width fits usize") * 4
    );

    let pixels = bytes[newline + 1..].to_vec();
    assert_eq!(
        pixels.len(),
        stride * usize::try_from(height).expect("height fits usize"),
        "capture payload size"
    );

    CapturedFrame {
        width,
        height,
        pixels,
    }
}
