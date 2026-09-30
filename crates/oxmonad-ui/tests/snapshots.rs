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
    PpStyle,
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
    join_groups,
    pp_segments,
    render_bar,
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
        segments_left: pp_segments(&tags, "Tall", title, &PpStyle::default()),
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
