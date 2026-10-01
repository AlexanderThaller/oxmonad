//! How long drawing each widget takes, to judge whether rendering is worth
//! optimizing. Run with `cargo bench -p oxmonad-ui`.

use std::{
    hint::black_box,
    time::Duration,
};

use criterion::{
    Criterion,
    criterion_group,
    criterion_main,
};
use oxmonad_ui::{
    Antialias,
    BarStyle,
    BarView,
    BatteryState,
    BatteryStatus,
    BatteryStyle,
    Fonts,
    HelpBinding,
    HelpGroup,
    HelpStyle,
    HelpView,
    Icon,
    MenuEntry,
    MenuEntryKind,
    MenuStyle,
    MenuView,
    PpStyle,
    Segment,
    WorkspaceState,
    WorkspaceTag,
    format_battery,
    join_groups,
    pp_segments,
    render_bar,
    render_help,
    render_menu,
};

/// The bar of a busy session: nine workspaces, a long title, battery,
/// three tray icons and the clock.
fn bar_view() -> BarView {
    let tags: Vec<WorkspaceTag> = (1..=9)
        .map(|n| WorkspaceTag {
            name: n.to_string(),
            state: match n {
                2 => WorkspaceState::Current,
                5 => WorkspaceState::Visible,
                _ => WorkspaceState::Hidden,
            },
            has_windows: n % 3 != 0,
            urgent: false,
        })
        .collect();
    let battery = BatteryState {
        percent: 80,
        status: BatteryStatus::Discharging,
        time_left: Some(Duration::from_mins(135)),
    };
    // A 64×64 icon is shrunk to the icon size like a real tray pixmap.
    let rgba: Vec<u8> = (0..=u8::MAX)
        .cycle()
        .take(64 * 64)
        .flat_map(|blue| [200, 40, blue, 255])
        .collect();
    let icon = Icon::from_straight_rgba(64, 64, &rgba).expect("valid icon size");
    let tray = (0..3)
        .map(|index| Segment::with_icon(icon.clone()).action(format!("item{index}")))
        .collect();
    BarView {
        segments_left: pp_segments(
            &tags,
            "Tall",
            "alacritty: ~/src/priv/oxmonad — cargo bench -p oxmonad-ui",
            &PpStyle::default(),
        ),
        segments_right: join_groups(
            [
                format_battery(battery, &BatteryStyle::default()),
                tray,
                vec![Segment::new("2026.10.01-4 13:05")],
            ],
            &BarStyle::default().separator,
        ),
    }
}

/// A help overlay about the size of the default configuration's.
fn help_view() -> HelpView {
    let groups = [
        "Launch",
        "Windows",
        "Layouts",
        "Workspaces",
        "Screens",
        "Session",
        "Mouse",
    ];
    HelpView {
        title: "Key bindings".to_owned(),
        hint: "Esc: close  j/k: scroll".to_owned(),
        groups: groups
            .iter()
            .map(|title| HelpGroup {
                title: (*title).to_owned(),
                bindings: (0..12)
                    .map(|n| HelpBinding {
                        combo: format!("Super+Shift+{n}"),
                        action: format!("{} action number {n}", title.to_lowercase()),
                    })
                    .collect(),
            })
            .collect(),
        scroll: 0,
    }
}

/// A tray menu like nm-applet's.
fn menu_view() -> MenuView {
    let mut entries: Vec<MenuEntry> = ["Open Window", "Notifications", "Do Not Disturb"]
        .into_iter()
        .map(MenuEntry::new)
        .collect();
    entries.push(MenuEntry::separator());
    for (label, on) in [("Wi-Fi", true), ("Ethernet", false), ("VPN", false)] {
        let mut entry = MenuEntry::new(label);
        entry.kind = MenuEntryKind::Radio(on);
        entries.push(entry);
    }
    entries.push(MenuEntry::separator());
    let mut preferences = MenuEntry::new("Preferences");
    preferences.submenu = true;
    entries.extend([preferences, MenuEntry::new("Quit")]);
    MenuView {
        entries,
        selected: Some(1),
    }
}

fn bench(c: &mut Criterion) {
    let fonts = Fonts::embedded();
    let bar = bar_view();
    let help = help_view();
    let menu = menu_view();

    for scale in [1, 2] {
        c.bench_function(&format!("bar 1920 scale {scale}"), |b| {
            b.iter(|| render_bar(black_box(&bar), &BarStyle::default(), &fonts, 1920, scale));
        });
    }
    let subpixel = BarStyle {
        antialias: Antialias::Rgb,
        ..BarStyle::default()
    };
    c.bench_function("bar 1920 scale 1 subpixel", |b| {
        b.iter(|| render_bar(black_box(&bar), &subpixel, &fonts, 1920, 1));
    });
    for scale in [1, 2] {
        c.bench_function(&format!("help 1920x1080 scale {scale}"), |b| {
            b.iter(|| {
                render_help(
                    black_box(&help),
                    &HelpStyle::default(),
                    &fonts,
                    1920,
                    1080,
                    scale,
                )
            });
        });
        c.bench_function(&format!("menu scale {scale}"), |b| {
            b.iter(|| render_menu(black_box(&menu), &MenuStyle::default(), &fonts, scale));
        });
    }
}

criterion_group!(benches, bench);
criterion_main!(benches);
