use super::*;
use crate::{
    action::Direction,
    prompt::PromptKind,
};

fn key(s: &str, config: &Config) -> KeyCombo {
    KeyCombo::parse(s, config.modifier).expect("valid combo")
}

fn action(config: &Config, combo: &str) -> Option<Action> {
    config.key_action(&key(combo, config)).cloned()
}

fn load(user: &str) -> Config {
    Config::from_toml(user).expect("config loads")
}

#[test]
fn defaults_parse() {
    let config = Config::defaults();
    assert_eq!(config, Config::default());
    assert_eq!(config.modifier, Modifiers::SUPER);
    assert_eq!(config.terminal, "alacritty");
    assert!(config.focus_follows_mouse);
    assert!(!load("focus_follows_mouse = false").focus_follows_mouse);
    assert_eq!(
        config.workspaces,
        ["1", "2", "3", "4", "5", "6", "7", "8", "9"]
    );
    assert_eq!(config.border.width, 1);
    assert_eq!(config.border.focused, Color::rgb(0xC6, 0x28, 0x28));
    assert_eq!(config.border.normal, Color::rgb(0x96, 0x96, 0x94));
    assert_eq!(config.font.antialias, Antialias::Grayscale);
    assert_eq!(
        load("[font]\nantialias = \"bgr\"").font.antialias,
        Antialias::Bgr
    );
    assert!(Config::from_toml("[font]\nantialias = \"lcd\"").is_err());
    assert!(config.bar.visible);
    assert_eq!(config.bar.position, BarPosition::Top);
    assert_eq!(config.bar.output, None);
    assert_eq!(config.bar.primary, None);
    assert_eq!(config.bar.title_max_len, Some(100));
    assert!(config.bar.tray);
    assert_eq!(config.bar.tray_icon_size, 16);
    assert!(config.bar.layout_icon);
    assert!(!config.bar.show_empty_workspaces);
    assert_eq!(config.bar.colors.current, Color::rgb(0xF8, 0xE6, 0x3A));
    assert_eq!(config.bar.colors.title, Color::rgb(0x86, 0xBD, 0x47));
    assert_eq!(config.prompt.position, BarPosition::Bottom);
    assert_eq!(
        config.prompt.colors.highlight_background,
        Color::rgb(0x15, 0x65, 0xC1)
    );
    assert_eq!(config.tabs.height, 20);
    assert_eq!(config.tabs.active_color, Color::rgb(0x99, 0x99, 0x99));
    assert_eq!(config.rules.len(), 2);
}

#[test]
fn default_keybindings_cover_the_documented_set() {
    let c = Config::defaults();
    let spawn = |s: &str| Some(Action::Spawn(s.to_owned()));
    let expected = [
        ("Mod+Shift+Return", Some(Action::SpawnTerminal)),
        (
            "Mod+p",
            spawn(
                "tofi-run --anchor top --width 100% --height 30 --horizontal true --prompt-text \
                 'run: ' --font 'Fira Mono' --font-size 14 --background-color '#161616' \
                 --text-color '#F2F2F2' --selection-color '#F9A825' --border-width 0 \
                 --outline-width 0 --padding-top 0 --padding-left 8 --result-spacing 15 | xargs \
                 --no-run-if-empty setsid --fork",
            ),
        ),
        ("Mod+Shift+l", spawn("oxmonad-lock")),
        ("Mod+j", Some(Action::FocusNext)),
        ("Mod+k", Some(Action::FocusPrev)),
        ("Mod+Shift+j", Some(Action::SwapNext)),
        ("Mod+Shift+k", Some(Action::SwapPrev)),
        ("Mod+Return", Some(Action::SwapMaster)),
        ("Mod+t", Some(Action::Sink)),
        ("Mod+Shift+c", Some(Action::Close)),
        ("Mod+o", Some(Action::ShiftScreenFollow(Direction::Next))),
        ("Mod+space", Some(Action::NextLayout)),
        ("Mod+Shift+space", Some(Action::ResetLayout)),
        ("Mod+Ctrl+space", Some(Action::PickLayout)),
        ("Mod+h", Some(Action::Shrink)),
        ("Mod+l", Some(Action::Expand)),
        ("Mod+Up", Some(Action::MirrorExpand)),
        ("Mod+Down", Some(Action::MirrorShrink)),
        ("Mod+comma", Some(Action::IncMaster(1))),
        ("Mod+period", Some(Action::IncMaster(-1))),
        ("Mod+Shift+z", Some(Action::ToggleWs)),
        ("Mod+v", Some(Action::Prompt(PromptKind::ViewWorkspace))),
        ("Mod+m", Some(Action::Prompt(PromptKind::MoveToWorkspace))),
        (
            "Mod+Shift+m",
            Some(Action::Prompt(PromptKind::CopyToWorkspace)),
        ),
        (
            "Mod+Shift+r",
            Some(Action::Prompt(PromptKind::RenameWorkspace)),
        ),
        ("Mod+Shift+BackSpace", Some(Action::RemoveWorkspace)),
        ("Mod+Right", Some(Action::FocusScreen(Direction::Next))),
        ("Mod+Left", Some(Action::FocusScreen(Direction::Prev))),
        (
            "Mod+Shift+Right",
            Some(Action::ShiftScreen(Direction::Next)),
        ),
        ("Mod+Shift+Left", Some(Action::ShiftScreen(Direction::Prev))),
        ("Mod+z", Some(Action::SwapNextScreen)),
        ("Mod+Shift+s", Some(Action::SplitScreen(2))),
        ("Mod+Ctrl+Shift+s", Some(Action::Rescreen)),
        ("Mod+b", Some(Action::ToggleBar)),
        ("Mod+n", Some(Action::Refresh)),
        ("Mod+Shift+q", Some(Action::Quit)),
        ("Mod+q", Some(Action::Restart)),
        ("Mod+Shift+slash", Some(Action::Help)),
        ("Mod+f", Some(Action::ToggleFullscreen)),
        ("Mod+grave", Some(Action::ToggleScratchpad("term".into()))),
    ];
    let listed = expected.len();
    for (combo, want) in expected {
        assert_eq!(action(&c, combo), want, "{combo}");
    }
    for n in 1..=9 {
        assert_eq!(
            action(&c, &format!("Mod+{n}")),
            Some(Action::View(n.to_string()))
        );
        assert_eq!(
            action(&c, &format!("Mod+Shift+{n}")),
            Some(Action::Shift(n.to_string()))
        );
    }
    // 46 listed + 18 workspace bindings.
    assert_eq!(c.keybindings.len(), listed + 18);
}

#[test]
fn default_pointer_bindings() {
    let c = Config::defaults();
    assert_eq!(c.pointer_bindings.len(), 2);
    let find = |button| {
        c.pointer_bindings
            .iter()
            .find(|b| b.combo.button == button)
            .map(|b| (b.combo.mods, b.action.clone()))
    };
    assert_eq!(
        find(PointerButton::Left),
        Some((Modifiers::SUPER, Action::FloatMove))
    );
    assert_eq!(
        find(PointerButton::Right),
        Some((Modifiers::SUPER, Action::FloatResize))
    );
}

#[test]
fn default_scratchpad() {
    let c = Config::defaults();
    assert_eq!(c.scratchpads.len(), 1);
    let pad = &c.scratchpads[0];
    assert_eq!(pad.name, "term");
    assert_eq!(pad.app_id, "scratchpad");
    assert_eq!(
        pad.rect,
        RationalRect::new(
            Ratio::new(1, 10),
            Ratio::new(1, 10),
            Ratio::new(4, 5),
            Ratio::new(4, 5)
        )
    );
}

#[test]
fn modifier_applies_to_default_bindings() {
    let c = load(
        r#"
modifier = "Alt"
[keybindings]
"XF86AudioPlay" = "spawn playerctl play-pause"
"#,
    );
    assert_eq!(c.modifier, Modifiers::ALT);
    let alt_j = KeyCombo {
        mods: Modifiers::ALT,
        key: "j".into(),
    };
    let super_j = KeyCombo {
        mods: Modifiers::SUPER,
        key: "j".into(),
    };
    assert_eq!(c.key_action(&alt_j), Some(&Action::FocusNext));
    assert_eq!(c.key_action(&super_j), None);
    assert!(
        c.pointer_bindings
            .iter()
            .all(|b| b.combo.mods == Modifiers::ALT)
    );
    // Unmodified bindings stay unmodified.
    assert!(action(&c, "XF86AudioPlay").is_some());
}

#[test]
fn user_bindings_override_add_and_remove() {
    let c = load(
        r#"
        [keybindings]
        "Mod+Return" = "spawn-terminal"
        "Mod+Shift+Return" = "none"
        "Mod+Ctrl+e" = "spawn emacsclient -c"
        "#,
    );
    assert_eq!(action(&c, "Mod+Return"), Some(Action::SpawnTerminal));
    assert_eq!(action(&c, "Mod+Shift+Return"), None);
    assert_eq!(
        action(&c, "Ctrl+Mod+e"),
        Some(Action::Spawn("emacsclient -c".into()))
    );
    assert_eq!(c.keybindings.len(), Config::defaults().keybindings.len());
}

#[test]
fn aliases_collide_with_defaults() {
    let c = load(
        r#"
        [keybindings]
        "Super+j" = "focus-prev"
        "super+SHIFT+k" = "none"
        "#,
    );
    assert_eq!(action(&c, "Mod+j"), Some(Action::FocusPrev));
    assert_eq!(action(&c, "Mod+Shift+k"), None);
    let count = c
        .keybindings
        .iter()
        .filter(|b| b.combo.key == "j" && b.combo.mods == Modifiers::SUPER)
        .count();
    assert_eq!(count, 1);
}

#[test]
fn key_names_override_defaults_ignoring_case() {
    let c = load(
        r#"
        [keybindings]
        "Mod+Shift+J" = "none"
        "Mod+Shift+C" = "focus-master"
        "Mod+RETURN" = "spawn-terminal"
        "#,
    );
    assert_eq!(action(&c, "Mod+Shift+j"), None);
    assert_eq!(action(&c, "Mod+Shift+c"), Some(Action::FocusMaster));
    assert_eq!(action(&c, "Mod+Return"), Some(Action::SpawnTerminal));
    let shift_c = c
        .keybindings
        .iter()
        .filter(|b| {
            b.combo.key.to_lowercase() == "c" && b.combo.mods == Modifiers::SUPER | Modifiers::SHIFT
        })
        .count();
    assert_eq!(shift_c, 1);
    assert_eq!(
        c.keybindings.len(),
        Config::defaults().keybindings.len() - 1
    );
}

#[test]
fn pointer_bindings_merge() {
    let c = load(
        r#"
        [pointer_bindings]
        "Mod+Button3" = "none"
        "Mod+Button2" = "float-resize"
        "#,
    );
    let buttons: Vec<_> = c.pointer_bindings.iter().map(|b| b.combo.button).collect();
    assert_eq!(buttons, [PointerButton::Left, PointerButton::Middle]);
}

#[test]
fn bindings_are_sorted_deterministically() {
    let c = Config::defaults();
    let combos: Vec<_> = c.keybindings.iter().map(|b| b.combo.clone()).collect();
    let mut sorted = combos.clone();
    sorted.sort();
    assert_eq!(combos, sorted);
}

#[test]
fn nested_tables_deep_merge() {
    let c = load(
        r##"
        [bar.colors]
        title = "#FF0000"
        [bar]
        output = "DP-1"
        title_max_len = 0
        tray = false
        tray_icon_size = 24
        layout_icon = false
        show_empty_workspaces = true
        "##,
    );
    let d = Config::defaults();
    assert_eq!(c.bar.colors.title, Color::rgb(0xFF, 0, 0));
    assert_eq!(c.bar.colors.current, d.bar.colors.current);
    assert_eq!(c.bar.height, d.bar.height);
    // The string form loads as a one-element list.
    assert_eq!(c.bar.output, Some(vec!["DP-1".to_owned()]));
    assert_eq!(c.bar.title_max_len, None);
    assert!(!c.bar.tray);
    assert_eq!(c.bar.tray_icon_size, 24);
    assert!(!c.bar.layout_icon);
    assert!(c.bar.show_empty_workspaces);
    assert!(Config::from_toml("[bar]\ntray_icon_size = -1").is_err());
}

#[test]
fn bar_output_list_and_primary() {
    let c = load(
        r#"
        [bar]
        output = ["DP-2", "DP-1"]
        primary = "DP-1"
        "#,
    );
    assert_eq!(
        c.bar.output,
        Some(vec!["DP-2".to_owned(), "DP-1".to_owned()])
    );
    assert_eq!(c.bar.primary.as_deref(), Some("DP-1"));
    // An empty list is allowed: no bar at all.
    assert_eq!(load("[bar]\noutput = []").bar.output, Some(Vec::new()));
    // Anything else is rejected with a readable message.
    let error = Config::from_toml("[bar]\noutput = 1")
        .expect_err("not a name")
        .to_string();
    assert!(
        error.contains("an output name or a list of output names"),
        "{error}"
    );
    assert!(Config::from_toml("[bar]\noutput = [1]").is_err());
}

#[test]
fn arrays_replace_wholesale() {
    let c = load(
        r#"
        workspaces = ["web", "code"]
        rules = []
        [[scratchpads]]
        name = "music"
        command = "alacritty --class music -e ncmpcpp"
        app_id = "music"
        rect = [0.0, 0.5, 1.0, 0.5]
        "#,
    );
    assert_eq!(c.workspaces, ["web", "code"]);
    assert_eq!(c.rules, []);
    assert_eq!(c.scratchpads.len(), 1);
    assert_eq!(c.scratchpads[0].rect.y, Ratio::new(1, 2));
    assert_eq!(c.scratchpads[0].rect.w, Ratio::ONE);
}

#[test]
fn user_rules_parse() {
    let c = load(
        r#"
        [[rules]]
        app_id = "firefox"
        workspace = "2"
        [[rules]]
        title = "*Picture-in-Picture*"
        action = "float"
        "#,
    );
    assert_eq!(c.rules.len(), 2);
    assert!(c.rules[1].matches("firefox", "Firefox — Picture-in-Picture", false));
}

#[test]
fn error_cases() {
    let err = |user: &str| Config::from_toml(user).expect_err("config must be rejected");
    assert!(matches!(err("this is not toml"), ConfigError::Toml(_)));
    assert!(matches!(err("frobnicate = 1"), ConfigError::Toml(_)));
    assert!(matches!(err("[bar]\ncolour = 1"), ConfigError::Toml(_)));
    assert!(matches!(
        err("[border]\nfocused = \"red\""),
        ConfigError::Toml(_)
    ));
    assert!(matches!(
        err(r#"modifier = "Mod""#),
        ConfigError::InvalidModifier(KeyError::ModInModifier(_))
    ));
    assert!(matches!(
        err(r#"modifier = "Hyper""#),
        ConfigError::InvalidModifier(_)
    ));
    assert!(matches!(
        err("[keybindings]\n\"Hyper+j\" = \"quit\""),
        ConfigError::InvalidCombo { .. }
    ));
    assert_eq!(
        err("[keybindings]\n\"Mod+j\" = \"jump\""),
        ConfigError::InvalidAction {
            combo: "Mod+j".into(),
            source: ParseActionError::Unknown("jump".into()),
        }
    );
    assert!(matches!(
        err("[keybindings]\n\"Mod+j\" = 1"),
        ConfigError::InvalidBindingValue(_)
    ));
    assert!(matches!(
        err("keybindings = 1"),
        ConfigError::InvalidSection(_)
    ));
    assert!(matches!(
        err("[pointer_bindings]\n\"Mod+Button7\" = \"float-move\""),
        ConfigError::InvalidCombo { .. }
    ));
    assert!(matches!(
        err("workspaces = []"),
        ConfigError::EmptyWorkspaces
    ));
    assert!(matches!(
        err(r#"workspaces = ["1", "1"]"#),
        ConfigError::InvalidWorkspace(_)
    ));
    assert!(matches!(
        err("[border]\nwidth = -1"),
        ConfigError::NegativeSize("border.width")
    ));
    let pad = |rect: &str| {
        format!("[[scratchpads]]\nname = \"x\"\ncommand = \"x\"\napp_id = \"x\"\nrect = {rect}")
    };
    assert!(matches!(
        err(&pad("[0.0, 0.0, 1.5, 0.5]")),
        ConfigError::InvalidScratchpad { .. }
    ));
    assert!(matches!(
        err(&pad("[0.0, 0.0, 0.0, 0.5]")),
        ConfigError::InvalidScratchpad { .. }
    ));
    assert!(matches!(err(&pad("[0.0, 0.0, 0.5]")), ConfigError::Toml(_)));
}

#[test]
fn error_messages_mention_the_binding() {
    let message = Config::from_toml("[keybindings]\n\"Mod+j\" = \"jump\"")
        .expect_err("rejected")
        .to_string();
    assert!(message.contains("Mod+j"), "{message}");
    assert!(message.contains("jump"), "{message}");
}

#[test]
fn default_layouts_are_the_standard_ring() {
    let config = Config::defaults();
    assert_eq!(config.layouts, LayoutOrder::standard());
    assert_eq!(
        config.layouts.names(),
        [
            "Tall Right",
            "Tall",
            "Wide",
            "Full",
            "Tabs",
            "Grid Rows",
            "Grid",
            "Three Columns",
            "Three Columns Right",
            "Corner",
        ]
    );
}

#[test]
fn layouts_set_order_and_subset_ignoring_case() {
    let config = load(r#"layouts = ["grid", "TALL", " three columns right "]"#);
    assert_eq!(
        config.layouts.names(),
        ["Grid", "Tall", "Three Columns Right"]
    );
    // Other settings keep their defaults.
    assert_eq!(config.workspaces.len(), 9);
}

#[test]
fn invalid_layouts_are_rejected_with_the_valid_names() {
    let err = |user: &str| Config::from_toml(user).expect_err("config must be rejected");
    assert_eq!(
        err("layouts = []"),
        ConfigError::InvalidLayouts(LayoutOrderError::Empty)
    );
    assert_eq!(
        err(r#"layouts = ["Tall", "Spiral"]"#),
        ConfigError::InvalidLayouts(LayoutOrderError::Unknown("Spiral".into()))
    );
    assert_eq!(
        err(r#"layouts = ["Grid", "Full", "grid"]"#),
        ConfigError::InvalidLayouts(LayoutOrderError::Duplicate("grid".into()))
    );
    assert!(matches!(err("layouts = [1]"), ConfigError::Toml(_)));
    assert!(matches!(err(r#"layouts = "Tall""#), ConfigError::Toml(_)));

    let unknown = err(r#"layouts = ["Spiral"]"#).to_string();
    assert!(unknown.contains("`layouts`"), "{unknown}");
    assert!(unknown.contains("Spiral"), "{unknown}");
    for name in crate::layout::standard_names() {
        assert!(unknown.contains(&format!("\"{name}\"")), "{unknown}");
    }
    let empty = err("layouts = []").to_string();
    assert!(empty.contains("\"Three Columns Right\""), "{empty}");
    let duplicate = err(r#"layouts = ["Tabs", "tabs"]"#).to_string();
    assert!(duplicate.contains("`tabs`"), "{duplicate}");
}

#[test]
fn smart_borders() {
    use crate::config::SmartBorders;
    for (value, smart) in [
        ("off", SmartBorders::Off),
        ("single-screen", SmartBorders::SingleScreen),
        ("per-screen", SmartBorders::PerScreen),
    ] {
        let c = load(&format!("[border]\nsmart = \"{value}\""));
        assert_eq!(c.border.smart, smart);
        assert_eq!(c.border.width, Config::defaults().border.width);
    }
    assert!(Config::from_toml("[border]\nsmart = \"always\"").is_err());
    assert!(Config::from_toml("[border]\nsmart = true").is_err());
}

#[test]
fn split_divider() {
    let defaults = Config::defaults();
    assert_eq!(defaults.split.divider_width, 3);
    assert_eq!(defaults.split.divider_color, Color::rgb(0x1E, 0x88, 0xE5));
    assert_eq!(defaults.split.divider_padding, 2);

    let c = load("[split]\ndivider_width = 0\ndivider_padding = 0");
    assert_eq!(c.split.divider_width, 0);
    assert_eq!(c.split.divider_padding, 0);
    assert_eq!(c.split.divider_color, defaults.split.divider_color);
    let c = load("[split]\ndivider_width = 8\ndivider_color = \"#ff000080\"");
    assert_eq!(c.split.divider_width, 8);
    assert_eq!(
        c.split.divider_color,
        Color {
            a: 0x80,
            ..Color::rgb(0xFF, 0, 0)
        }
    );

    let err = |user: &str| Config::from_toml(user).expect_err("config must be rejected");
    assert!(matches!(
        err("[split]\ndivider_width = -1"),
        ConfigError::NegativeSize("split.divider_width")
    ));
    assert!(matches!(
        err("[split]\ndivider_padding = -1"),
        ConfigError::NegativeSize("split.divider_padding")
    ));
    assert!(matches!(
        err("[split]\ndivider_width = 1.5"),
        ConfigError::Toml(_)
    ));
    assert!(matches!(
        err("[split]\ndivider_color = \"blue\""),
        ConfigError::Toml(_)
    ));
    assert!(matches!(err("[split]\ngap = 2"), ConfigError::Toml(_)));
}
