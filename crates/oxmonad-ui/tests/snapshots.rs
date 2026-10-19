//! Golden-image tests. Regenerate with `OXMONAD_BLESS=1 cargo test -p
//! oxmonad-ui`; on mismatch the render is written to `<name>.actual.png`.

use std::{
    path::PathBuf,
    time::Duration,
};

use oxmonad_ui::{
    Antialias,
    BarStyle,
    BarView,
    BatteryState,
    BatteryStatus,
    BatteryStyle,
    Canvas,
    Fonts,
    HelpBinding,
    HelpGroup,
    HelpStyle,
    HelpView,
    Icon,
    LayoutGlyph,
    LayoutPickerStyle,
    LayoutPickerView,
    MenuEntry,
    MenuEntryKind,
    MenuStyle,
    MenuView,
    PickerEntry,
    PpStyle,
    PreviewRect,
    PreviewWindow,
    PromptPosition,
    PromptStyle,
    PromptView,
    Segment,
    Tab,
    TabStyle,
    TabsView,
    WorkspaceState,
    WorkspaceTag,
    format_battery,
    help_size,
    help_viewport,
    join_groups,
    layout_picker_size,
    pp_segments,
    render_bar,
    render_fill,
    render_help,
    render_layout_picker,
    render_menu,
    render_prompt,
    render_tabs,
};

/// Converts premultiplied BGRA to straight RGBA.
fn to_rgba(canvas: &Canvas) -> image::RgbaImage {
    let mut pixels = canvas.as_bytes().to_vec();
    for pixel in pixels.as_chunks_mut::<4>().0 {
        let [b, g, r, a] = [pixel[0], pixel[1], pixel[2], pixel[3]];
        let unpremultiply = |c: u8| {
            if a == 0 {
                0
            } else {
                u8::try_from((u32::from(c) * 255 + u32::from(a) / 2) / u32::from(a))
                    .unwrap_or(u8::MAX)
            }
        };
        pixel.copy_from_slice(&[unpremultiply(r), unpremultiply(g), unpremultiply(b), a]);
    }
    image::RgbaImage::from_raw(canvas.width(), canvas.height(), pixels)
        .expect("buffer matches the canvas size")
}

fn assert_snapshot(name: &str, canvas: &Canvas) {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots");
    let path = dir.join(format!("{name}.png"));
    let actual = to_rgba(canvas);
    if std::env::var_os("OXMONAD_BLESS").is_some_and(|value| value == "1") {
        actual.save(&path).expect("write snapshot");
        return;
    }
    let expected = image::open(&path)
        .unwrap_or_else(|error| panic!("{}: {error}; bless with OXMONAD_BLESS=1", path.display()))
        .into_rgba8();
    let differing = if expected.dimensions() == actual.dimensions() {
        expected
            .pixels()
            .zip(actual.pixels())
            .filter(|(a, b)| a != b)
            .count()
    } else {
        usize::MAX
    };
    if differing != 0 {
        let actual_path = dir.join(format!("{name}.actual.png"));
        actual.save(&actual_path).expect("write actual render");
        panic!(
            "{name}: {} differs from the render ({:?} vs {:?}, {differing} pixels differ); see {}",
            path.display(),
            expected.dimensions(),
            actual.dimensions(),
            actual_path.display()
        );
    }
}

fn tag(name: &str, state: WorkspaceState, has_windows: bool, urgent: bool) -> WorkspaceTag {
    WorkspaceTag {
        name: name.to_owned(),
        state,
        has_windows,
        urgent,
    }
}

fn bar_view(title: &str, battery: BatteryState) -> BarView {
    let tags = [
        tag("1", WorkspaceState::Hidden, true, false),
        tag("2", WorkspaceState::Current, true, false),
        tag("3", WorkspaceState::Hidden, false, false),
        tag("4", WorkspaceState::Visible, true, false),
        tag("5", WorkspaceState::Hidden, true, true),
        tag("web", WorkspaceState::Hidden, true, false),
    ];
    let style = BarStyle::default();
    BarView {
        segments_left: pp_segments(&tags, "Tall", None, title, &PpStyle::default()),
        segments_right: join_groups(
            [
                format_battery(battery, &BatteryStyle::default()),
                vec![Segment::new("2026.09.29-2 14:05")],
            ],
            &style.separator,
        ),
    }
}

fn discharging(percent: u8) -> BatteryState {
    BatteryState {
        percent,
        status: BatteryStatus::Discharging,
        time_left: Some(Duration::from_mins(135)),
    }
}

#[test]
fn bar_states() {
    let fonts = Fonts::embedded();
    let style = BarStyle::default();
    let cases = [
        ("bar_high", "alacritty — ~/src/oxmonad", discharging(93)),
        (
            "bar_normal",
            "Firefox: Übersicht — ÄÖÜ ß ñ €",
            discharging(55),
        ),
        ("bar_low", "", discharging(7)),
        (
            "bar_charging",
            "emacs@host: a very long window title that goes on and on and certainly will not fit \
             into the bar",
            BatteryState {
                percent: 40,
                status: BatteryStatus::Charging,
                time_left: None,
            },
        ),
        (
            "bar_charged",
            "zsh",
            BatteryState {
                percent: 100,
                status: BatteryStatus::Full,
                time_left: None,
            },
        ),
    ];
    for (name, title, battery) in cases {
        assert_snapshot(
            name,
            &render_bar(&bar_view(title, battery), &style, &fonts, 1100, 1),
        );
    }
}

#[test]
fn bar_scale_2() {
    let view = bar_view("alacritty", discharging(55));
    let canvas = render_bar(&view, &BarStyle::default(), &Fonts::embedded(), 800, 2);
    assert_eq!((canvas.width(), canvas.height()), (1600, 48));
    assert_snapshot("bar_scale_2", &canvas);
}

#[test]
fn bar_subpixel() {
    let view = bar_view("alacritty — ~/src/oxmonad", discharging(93));
    for (name, antialias) in [("bar_rgb", Antialias::Rgb), ("bar_bgr", Antialias::Bgr)] {
        let style = BarStyle {
            antialias,
            ..BarStyle::default()
        };
        assert_snapshot(
            name,
            &render_bar(&view, &style, &Fonts::embedded(), 1100, 1),
        );
    }
}

fn workspaces() -> Vec<String> {
    [
        "1", "2", "3", "4", "5", "6", "7", "8", "9", "web", "mail", "music", "chat", "scratch",
    ]
    .map(str::to_owned)
    .to_vec()
}

#[test]
fn prompt_without_completions() {
    let view = PromptView {
        label: "Workspace: ".to_owned(),
        input: "mail".to_owned(),
        cursor: 2,
        completions: Vec::new(),
        selected: None,
    };
    let canvas = render_prompt(
        &view,
        &PromptStyle::default(),
        &Fonts::embedded(),
        600,
        400,
        1,
    );
    assert_snapshot("prompt_plain", &canvas);
}

#[test]
fn prompt_with_completions() {
    let fonts = Fonts::embedded();
    let view = PromptView {
        label: "Workspace: ".to_owned(),
        input: String::new(),
        cursor: 0,
        completions: workspaces(),
        selected: None,
    };
    let style = PromptStyle::default();
    assert_snapshot(
        "prompt_completions",
        &render_prompt(&view, &style, &fonts, 600, 400, 1),
    );

    let selected = PromptView {
        input: "m".to_owned(),
        cursor: 1,
        completions: vec!["mail".to_owned(), "music".to_owned()],
        selected: Some(1),
        ..view.clone()
    };
    assert_snapshot(
        "prompt_selected",
        &render_prompt(&selected, &style, &fonts, 600, 400, 1),
    );

    let top = PromptStyle {
        position: PromptPosition::Top,
        max_rows: Some(2),
        ..style
    };
    let paged = PromptView {
        selected: Some(12),
        ..view
    };
    assert_snapshot(
        "prompt_top_paged",
        &render_prompt(&paged, &top, &fonts, 600, 400, 1),
    );
}

fn tabs(titles: &[&str], focused: usize) -> TabsView {
    TabsView {
        tabs: titles
            .iter()
            .enumerate()
            .map(|(index, title)| Tab {
                title: (*title).to_owned(),
                focused: index == focused,
                urgent: false,
            })
            .collect(),
    }
}

#[test]
fn tab_strips() {
    let fonts = Fonts::embedded();
    let style = TabStyle::default();
    assert_snapshot(
        "tabs_one",
        &render_tabs(&tabs(&["alacritty"], 0), &style, &fonts, 600, 1),
    );
    assert_snapshot(
        "tabs_three",
        &render_tabs(
            &tabs(
                &[
                    "alacritty",
                    "Firefox — a long page title that has to be ellipsized",
                    "emacs",
                ],
                1,
            ),
            &style,
            &fonts,
            600,
            1,
        ),
    );
    let mut many = tabs(
        &[
            "1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "11", "12",
        ],
        4,
    );
    many.tabs[8].urgent = true;
    assert_snapshot("tabs_many", &render_tabs(&many, &style, &fonts, 600, 1));
    // Subpixel text on the opaque tabs, grayscale on the translucent one.
    let mut translucent = style.clone();
    translucent.antialias = Antialias::Rgb;
    translucent.inactive.fill = oxmonad_ui::Color::rgba(0x66, 0x66, 0x66, 0xC0);
    assert_snapshot(
        "tabs_rgb_translucent",
        &render_tabs(
            &tabs(&["alacritty", "emacs", "Firefox"], 1),
            &translucent,
            &fonts,
            600,
            1,
        ),
    );
    assert_snapshot(
        "tabs_scale_2",
        &render_tabs(&tabs(&["alacritty", "emacs"], 0), &style, &fonts, 300, 2),
    );
}

/// The divider line between virtual screens: a solid fill at the device
/// scale (the default 3px wide `#1E88E5` line, and a translucent one).
#[test]
fn divider_fills() {
    let blue = oxmonad_ui::Color::rgba(0x1E, 0x88, 0xE5, 0xFF);
    let canvas = render_fill(3, 40, 2, blue);
    assert_eq!((canvas.width(), canvas.height()), (6, 80));
    assert_eq!(canvas.pixel(5, 79), Some([0xE5, 0x88, 0x1E, 0xFF]));
    assert_snapshot("divider_scale_2", &canvas);
    let translucent = oxmonad_ui::Color::rgba(0xFF, 0, 0, 0x80);
    assert_snapshot("divider_translucent", &render_fill(4, 20, 1, translucent));
}

/// The standard ring's previews (`Layout::preview` in oxmonad-core), so the
/// UI crate stays independent of core.
#[expect(clippy::type_complexity, reason = "a literal table")]
const LAYOUTS: &[(
    &str,
    &[(usize, i32, i32, i32, i32)],
    &[(i32, i32, i32, i32)],
)] = &[
    (
        "Tall Right",
        &[
            (0, 128, 0, 192, 180),
            (1, 0, 0, 128, 60),
            (2, 0, 60, 128, 60),
            (3, 0, 120, 128, 60),
        ],
        &[],
    ),
    (
        "Tall",
        &[
            (0, 0, 0, 192, 180),
            (1, 192, 0, 128, 60),
            (2, 192, 60, 128, 60),
            (3, 192, 120, 128, 60),
        ],
        &[],
    ),
    (
        "Wide",
        &[
            (0, 0, 0, 320, 108),
            (1, 0, 108, 106, 72),
            (2, 106, 108, 107, 72),
            (3, 213, 108, 107, 72),
        ],
        &[],
    ),
    ("Full", &[(0, 0, 0, 320, 180)], &[]),
    (
        "Tabs",
        &[(0, 0, 20, 320, 160)],
        &[
            (0, 0, 80, 20),
            (80, 0, 80, 20),
            (160, 0, 80, 20),
            (240, 0, 80, 20),
        ],
    ),
    (
        "Grid Rows",
        &[
            (0, 0, 0, 80, 180),
            (1, 80, 0, 80, 180),
            (2, 160, 0, 80, 180),
            (3, 240, 0, 80, 180),
        ],
        &[],
    ),
    (
        "Grid",
        &[
            (0, 0, 0, 160, 90),
            (1, 0, 90, 160, 90),
            (2, 160, 0, 160, 90),
            (3, 160, 90, 160, 90),
        ],
        &[],
    ),
    (
        "Three Columns",
        &[
            (0, 0, 0, 160, 180),
            (1, 160, 0, 80, 90),
            (2, 160, 90, 80, 90),
            (3, 240, 0, 80, 180),
        ],
        &[],
    ),
    (
        "Three Columns Right",
        &[
            (0, 160, 0, 160, 180),
            (1, 80, 0, 80, 90),
            (2, 80, 90, 80, 90),
            (3, 0, 0, 80, 180),
        ],
        &[],
    ),
    (
        "Corner",
        &[
            (0, 0, 0, 213, 120),
            (1, 213, 0, 107, 90),
            (2, 213, 90, 107, 90),
            (3, 0, 120, 213, 60),
        ],
        &[],
    ),
    (
        "Corner Right",
        &[
            (0, 107, 0, 213, 120),
            (1, 0, 0, 107, 90),
            (2, 0, 90, 107, 90),
            (3, 107, 120, 213, 60),
        ],
        &[],
    ),
];

fn picker(selected: usize, active: usize) -> LayoutPickerView {
    LayoutPickerView {
        title: "Layout of workspace 1".to_owned(),
        hint: "j/k: select  Return: apply  Esc: cancel".to_owned(),
        preview_size: (320, 180),
        entries: LAYOUTS
            .iter()
            .map(|(name, windows, tabs)| PickerEntry {
                name: (*name).to_owned(),
                windows: windows
                    .iter()
                    .map(|&(index, x, y, w, h)| PreviewWindow {
                        index,
                        rect: PreviewRect::new(x, y, w, h),
                    })
                    .collect(),
                tabs: tabs
                    .iter()
                    .map(|&(x, y, w, h)| PreviewRect::new(x, y, w, h))
                    .collect(),
            })
            .collect(),
        selected,
        active,
    }
}

#[test]
fn layout_pickers() {
    let fonts = Fonts::embedded();
    let style = LayoutPickerStyle::default();
    for (name, view, (width, height), scale) in [
        ("picker_first", picker(0, 0), (1920, 1080), 1),
        ("picker_middle", picker(6, 1), (1920, 1080), 1),
        ("picker_small_screen", picker(4, 4), (800, 600), 1),
        ("picker_scale_2", picker(2, 0), (1920, 1080), 2),
    ] {
        let canvas = render_layout_picker(&view, &style, &fonts, width, height, scale);
        let (w, h) = layout_picker_size(&view, &style, width, height);
        assert_eq!((canvas.width(), canvas.height()), (w * scale, h * scale));
        assert_snapshot(name, &canvas);
    }
}

/// The default bindings, grouped like core's `help_groups`.
const HELP_GROUPS: &[(&str, &[(&str, &str)])] = &[
    (
        "Launch",
        &[
            ("Super+Shift+Return", "spawn-terminal"),
            (
                "Super+p",
                "spawn tofi-run --anchor top --width 100% --height 30 | xargs --no-run-if-empty \
                 setsid --fork",
            ),
            ("Super+Shift+l", "spawn swaylock"),
            ("Super+grave", "toggle-scratchpad term"),
        ],
    ),
    (
        "Windows",
        &[
            ("Super+j", "focus-next"),
            ("Super+k", "focus-prev"),
            ("Super+Shift+j", "swap-next"),
            ("Super+Shift+k", "swap-prev"),
            ("Super+Return", "swap-master"),
            ("Super+t", "sink"),
            ("Super+f", "toggle-fullscreen"),
            ("Super+Shift+c", "close"),
        ],
    ),
    (
        "Layouts",
        &[
            ("Super+space", "next-layout"),
            ("Super+Shift+space", "reset-layout"),
            ("Super+Ctrl+space", "pick-layout"),
            ("Super+h", "shrink"),
            ("Super+l", "expand"),
            ("Super+Down", "mirror-shrink"),
            ("Super+Up", "mirror-expand"),
            ("Super+comma", "inc-master 1"),
            ("Super+period", "inc-master -1"),
        ],
    ),
    (
        "Workspaces",
        &[
            ("Super+1", "view 1"),
            ("Super+2", "view 2"),
            ("Super+3", "view 3"),
            ("Super+4", "view 4"),
            ("Super+5", "view 5"),
            ("Super+6", "view 6"),
            ("Super+7", "view 7"),
            ("Super+8", "view 8"),
            ("Super+9", "view 9"),
            ("Super+Shift+1", "shift 1"),
            ("Super+Shift+2", "shift 2"),
            ("Super+Shift+3", "shift 3"),
            ("Super+Shift+4", "shift 4"),
            ("Super+Shift+5", "shift 5"),
            ("Super+Shift+6", "shift 6"),
            ("Super+Shift+7", "shift 7"),
            ("Super+Shift+8", "shift 8"),
            ("Super+Shift+9", "shift 9"),
            ("Super+Shift+z", "toggle-ws"),
            ("Super+m", "prompt move"),
            ("Super+v", "prompt view"),
            ("Super+Shift+m", "prompt copy"),
            ("Super+Shift+r", "prompt rename"),
            ("Super+Shift+BackSpace", "remove-workspace"),
        ],
    ),
    (
        "Screens",
        &[
            ("Super+Left", "focus-screen prev"),
            ("Super+Right", "focus-screen next"),
            ("Super+Shift+Left", "shift-screen prev"),
            ("Super+Shift+Right", "shift-screen next"),
            ("Super+o", "shift-screen-follow next"),
            ("Super+z", "swap-next-screen"),
            ("Super+Shift+s", "split-screen 2"),
            ("Super+Ctrl+Shift+s", "rescreen"),
        ],
    ),
    (
        "Session",
        &[
            ("Super+Shift+slash", "help"),
            ("Super+b", "toggle-bar"),
            ("Super+n", "refresh"),
            ("Super+q", "restart"),
            ("Super+Shift+q", "quit"),
        ],
    ),
    (
        "Mouse",
        &[
            ("Super+Button1", "float-move"),
            ("Super+Button3", "float-resize"),
        ],
    ),
];

fn help(scroll: usize) -> HelpView {
    HelpView {
        title: "Key bindings".to_owned(),
        hint: "Esc: close  j/k: scroll".to_owned(),
        groups: HELP_GROUPS
            .iter()
            .map(|(title, bindings)| HelpGroup {
                title: (*title).to_owned(),
                bindings: bindings
                    .iter()
                    .map(|(combo, action)| HelpBinding {
                        combo: (*combo).to_owned(),
                        action: (*action).to_owned(),
                    })
                    .collect(),
            })
            .collect(),
        scroll,
    }
}

#[test]
fn help_overlays() {
    let fonts = Fonts::embedded();
    let style = HelpStyle::default();
    for (name, view, (width, height), scale, scrolls) in [
        ("help_full_hd", help(0), (1920, 1080), 1, false),
        ("help_scale_2", help(0), (1920, 1080), 2, false),
        ("help_small_screen", help(0), (800, 480), 1, true),
        ("help_small_screen_scrolled", help(12), (800, 480), 1, true),
    ] {
        let canvas = render_help(&view, &style, &fonts, width, height, scale);
        let (w, h) = help_size(&view, &style, &fonts, width, height, scale);
        assert_eq!((canvas.width(), canvas.height()), (w * scale, h * scale));
        assert!(w <= width && h <= height, "{name}: {w}x{h}");
        let (_, max_scroll) = help_viewport(&view, &style, &fonts, width, height, scale);
        assert_eq!(max_scroll > 0, scrolls, "{name}");
        assert_snapshot(name, &canvas);
    }
}

/// A synthetic straight-RGBA icon: `pixel(x, y)` at each pixel centre,
/// with coordinates in 0..1.
fn synthetic_icon(width: u32, height: u32, pixel: impl Fn(f32, f32) -> [u8; 4]) -> Icon {
    let mut rgba = Vec::new();
    for y in 0..height {
        for x in 0..width {
            #[expect(clippy::cast_precision_loss, reason = "tiny test icons")]
            let (fx, fy) = (
                (x as f32 + 0.5) / width as f32,
                (y as f32 + 0.5) / height as f32,
            );
            rgba.extend(pixel(fx, fy));
        }
    }
    Icon::from_straight_rgba(width, height, &rgba).expect("valid icon size")
}

/// A 64×64 red disc (shrunk), a 16×16 translucent green square with an
/// opaque border (drawn 1:1) and a 12×6 blue–white bar (enlarged).
fn tray_icons() -> Vec<Icon> {
    let disc = synthetic_icon(64, 64, |x, y| {
        let distance = (x - 0.5).hypot(y - 0.5);
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "clamped to 0..=255"
        )]
        let alpha = ((0.45 - distance) * 64.0 * 255.0).clamp(0.0, 255.0) as u8;
        [230, 40, 40, alpha]
    });
    let square = synthetic_icon(16, 16, |x, y| {
        let edge = x < 0.1 || x > 0.9 || y < 0.1 || y > 0.9;
        if edge {
            [60, 200, 80, 255]
        } else {
            [60, 200, 80, 110]
        }
    });
    let bar = synthetic_icon(12, 6, |x, _| {
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "0..=255"
        )]
        let level = (x * 255.0) as u8;
        [level, level, 255, 255]
    });
    vec![disc, square, bar]
}

fn tray_bar_view() -> BarView {
    let style = BarStyle::default();
    let tray = tray_icons()
        .into_iter()
        .enumerate()
        .map(|(index, icon)| Segment::with_icon(icon).action(format!("item{index}")))
        .collect();
    BarView {
        segments_left: pp_segments(
            &[tag("1", WorkspaceState::Current, true, false)],
            "Tall",
            None,
            "alacritty",
            &PpStyle::default(),
        ),
        segments_right: join_groups(
            [
                format_battery(discharging(80), &BatteryStyle::default()),
                tray,
                vec![Segment::new("2026.10.01-4 13:05")],
            ],
            &style.separator,
        ),
    }
}

#[test]
fn bar_with_tray() {
    let fonts = Fonts::embedded();
    let style = BarStyle::default();
    assert_snapshot(
        "bar_tray",
        &render_bar(&tray_bar_view(), &style, &fonts, 700, 1),
    );
    assert_snapshot(
        "bar_tray_scale_2",
        &render_bar(&tray_bar_view(), &style, &fonts, 700, 2),
    );
}

fn menu(selected: Option<usize>) -> MenuView {
    let entry = |label: &str, kind, enabled, submenu| MenuEntry {
        label: label.to_owned(),
        kind,
        enabled,
        submenu,
    };
    MenuView {
        entries: vec![
            entry("Open Window", MenuEntryKind::Normal, true, false),
            entry("Unavailable", MenuEntryKind::Normal, false, false),
            MenuEntry::separator(),
            entry("Notifications", MenuEntryKind::Check(true), true, false),
            entry("Do Not Disturb", MenuEntryKind::Check(false), true, false),
            entry("Locked", MenuEntryKind::Check(true), false, false),
            MenuEntry::separator(),
            entry("Wi-Fi", MenuEntryKind::Radio(true), true, false),
            entry("Ethernet", MenuEntryKind::Radio(false), true, false),
            MenuEntry::separator(),
            entry("Preferences", MenuEntryKind::Normal, true, true),
            entry("Quit", MenuEntryKind::Normal, true, false),
        ],
        selected,
    }
}

#[test]
fn menus() {
    let fonts = Fonts::embedded();
    let style = MenuStyle::default();
    assert_snapshot(
        "menu_selected",
        &render_menu(&menu(Some(4)), &style, &fonts, 1),
    );
    assert_snapshot(
        "menu_submenu_selected",
        &render_menu(&menu(Some(10)), &style, &fonts, 1),
    );
    assert_snapshot(
        "menu_scale_2",
        &render_menu(&menu(Some(7)), &style, &fonts, 2),
    );
    let narrow = MenuStyle {
        max_width: 120,
        ..MenuStyle::default()
    };
    let plain = MenuView {
        entries: vec![
            MenuEntry::new("A rather long menu entry label"),
            MenuEntry::new("Short"),
        ],
        selected: None,
    };
    assert_snapshot("menu_ellipsized", &render_menu(&plain, &narrow, &fonts, 1));
}

/// A layout miniature of `windows` (index, x, y, w, h) and `tabs` in the
/// 320×180 preview area.
fn glyph(windows: &[(usize, i32, i32, i32, i32)], tabs: &[(i32, i32, i32, i32)]) -> LayoutGlyph {
    LayoutGlyph {
        area: (320, 180),
        windows: windows
            .iter()
            .map(|&(index, x, y, w, h)| PreviewWindow {
                index,
                rect: PreviewRect::new(x, y, w, h),
            })
            .collect(),
        tabs: tabs
            .iter()
            .map(|&(x, y, w, h)| PreviewRect::new(x, y, w, h))
            .collect(),
    }
}

#[test]
fn bar_layout_icons() {
    let fonts = Fonts::embedded();
    let style = BarStyle::default();
    let tall = glyph(
        &[
            (0, 0, 0, 192, 180),
            (1, 192, 0, 128, 60),
            (2, 192, 60, 128, 60),
            (3, 192, 120, 128, 60),
        ],
        &[],
    );
    let grid = glyph(
        &[
            (0, 0, 0, 160, 90),
            (1, 160, 0, 160, 90),
            (2, 0, 90, 160, 90),
            (3, 160, 90, 160, 90),
        ],
        &[],
    );
    let tabs = glyph(
        &[(0, 0, 20, 320, 160)],
        &[
            (0, 0, 80, 20),
            (80, 0, 80, 20),
            (160, 0, 80, 20),
            (240, 0, 80, 20),
        ],
    );
    let full = glyph(&[(0, 0, 0, 320, 180)], &[]);
    for (name, layout, preview) in [
        ("tall", "Tall", tall),
        ("grid", "Grid", grid),
        ("tabs", "Tabs", tabs),
        ("full", "Full", full),
    ] {
        let view = BarView {
            segments_left: pp_segments(
                &[
                    tag("1", WorkspaceState::Current, true, false),
                    tag("2", WorkspaceState::Hidden, true, false),
                ],
                layout,
                Some(preview),
                "alacritty",
                &PpStyle::default(),
            ),
            segments_right: vec![Segment::new("2026.10.01-4 13:05")],
        };
        assert_snapshot(
            &format!("bar_layout_{name}"),
            &render_bar(&view, &style, &fonts, 500, 1),
        );
        assert_snapshot(
            &format!("bar_layout_{name}_scale_2"),
            &render_bar(&view, &style, &fonts, 500, 2),
        );
    }
}
