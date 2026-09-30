use super::*;
use crate::config::Color;

fn focused_color() -> Color {
    Config::defaults().border.focused
}

fn normal_color() -> Color {
    Config::defaults().border.normal
}

#[test]
fn empty_wm_arranges_nothing() {
    let wm = wm();
    let a = wm.arrange();
    assert_eq!(a.windows.len(), 0);
    assert_eq!(a.hidden.len(), 0);
    assert_eq!(a.focus, Focus::None);
    assert_eq!(a.screens.len(), 1);
    assert_eq!(a.prompt, None);
}

#[test]
fn headless_wm_hides_everything() {
    let mut wm = Wm::new(Config::defaults());
    manage_all(&mut wm, [1]);
    let a = wm.arrange();
    assert_eq!(a.windows.len(), 0);
    assert_eq!(a.hidden, [w(1)]);
    assert_eq!(a.focus, Focus::Window(w(1)));
    assert_eq!(a.screens.len(), 0);
}

#[test]
fn default_layout_reflects_tall_with_borders() {
    let mut wm = wm();
    manage_all(&mut wm, [1, 2]);
    let a = wm.arrange();
    // Tall Right: master (2) on the right 3/5, stack on the left.
    let master = a.placement(w(2)).expect("placed");
    assert_eq!(master.rect, Rect::new(769, 25, 1150, 1054));
    assert_eq!(master.layer, Layer::Tiled);
    assert_eq!(
        master.border,
        Some(Border {
            width: 1,
            color: focused_color()
        })
    );
    let stack = a.placement(w(1)).expect("placed");
    assert_eq!(stack.rect, Rect::new(1, 25, 766, 1054));
    assert_eq!(stack.border.map(|b| b.color), Some(normal_color()));
    assert_eq!(a.focus, Focus::Window(w(2)));
    assert_eq!(a.hidden.len(), 0);
}

#[test]
fn full_layout_shows_only_focus_without_borders() {
    let mut wm = wm();
    manage_all(&mut wm, [1, 2, 3]);
    for _ in 0..3 {
        apply(&mut wm, "next-layout");
    }
    assert_eq!(wm.bar_model().layout, "Full");
    let a = wm.arrange();
    assert_eq!(a.windows.len(), 1);
    let p = &a.windows[0];
    assert_eq!((p.window, p.rect, p.border), (w(3), AREA, None));
    assert_eq!(a.hidden, [w(1), w(2)]);
}

#[test]
fn tabbed_layout_exposes_tab_strip() {
    let mut wm = wm();
    manage_all(&mut wm, [1, 2]);
    for _ in 0..4 {
        apply(&mut wm, "next-layout");
    }
    assert_eq!(wm.bar_model().layout, "Tabs");
    let a = wm.arrange();
    assert_eq!(a.windows.len(), 1);
    // Below the 20px strip, with the 1px border inside the slot.
    assert_eq!(a.windows[0].rect, Rect::new(1, 45, 1918, 1034));
    let strip = a.screens[0].tab_strip.clone().expect("tabs shown");
    assert_eq!(strip.rect, Rect::new(0, 24, 1920, 20));
    assert_eq!(
        strip.tabs,
        [
            Tab {
                window: w(2),
                title: "window 2".into(),
                focused: true,
                rect: Rect::new(0, 24, 960, 20),
            },
            Tab {
                window: w(1),
                title: "window 1".into(),
                focused: false,
                rect: Rect::new(960, 24, 960, 20),
            },
        ]
    );
    // A single window hides the strip (WhenPlural).
    wm.unmanage(w(1));
    assert_eq!(wm.arrange().screens[0].tab_strip, None);
}

#[test]
fn floating_windows_stack_above_tiled_focused_on_top() {
    let mut wm = wm();
    manage_all(&mut wm, [1, 2, 3]);
    wm.float_at(w(1), Rect::new(100, 100, 200, 200));
    wm.float_at(w(2), Rect::new(150, 150, 200, 200));
    let a = wm.arrange();
    let layers: Vec<(WindowId, Layer)> = a.windows.iter().map(|p| (p.window, p.layer)).collect();
    // Stack is 3 2 1; floats bottom-to-top in reverse stack order: 1, 2.
    assert_eq!(
        layers,
        [
            (w(3), Layer::Tiled),
            (w(1), Layer::Floating),
            (w(2), Layer::Floating),
        ]
    );
    assert_eq!(rect_of(&a, 1), Some(Rect::new(100, 100, 200, 200)));
    // The lone tiled window gets the whole area.
    assert_eq!(rect_of(&a, 3), Some(AREA.inset(1)));
    wm.focus_window(w(1));
    let a = wm.arrange();
    assert_eq!(a.windows.last().map(|p| p.window), Some(w(1)));
}

#[test]
fn floats_are_relative_to_their_screen() {
    let mut wm = wm2();
    manage_all(&mut wm, [1]);
    wm.float_at(w(1), Rect::new(100, 100, 200, 100));
    // Swap the workspace onto the second (same size) screen.
    apply(&mut wm, "swap-next-screen");
    assert_eq!(
        rect_of(&wm.arrange(), 1),
        Some(Rect::new(2020, 100, 200, 100))
    );
    // A smaller screen scales the float.
    wm.set_output_rect(O1, Rect::new(1920, 0, 960, 540));
    let frame = rect_of(&wm.arrange(), 1).map(|r| r.outset(1));
    assert_eq!(frame, Some(Rect::new(1969, 49, 101, 51)));
}

#[test]
fn fullscreen_covers_whole_screen_on_top() {
    let mut wm = wm();
    manage_all(&mut wm, [1, 2]);
    wm.request_fullscreen(w(1), true);
    let a = wm.arrange();
    let last = a.windows.last().expect("windows");
    assert_eq!(last.window, w(1));
    assert_eq!(last.rect, SCREEN);
    assert_eq!(last.layer, Layer::Fullscreen);
    assert_eq!(last.border, None);
    // The other window tiles alone.
    assert_eq!(rect_of(&a, 2), Some(AREA.inset(1)));
    wm.request_fullscreen(w(1), false);
    assert_eq!(
        wm.arrange().placement(w(1)).map(|p| p.layer),
        Some(Layer::Tiled)
    );
    // Requests for unknown windows are ignored.
    wm.request_fullscreen(w(9), true);
    assert!(!wm.is_fullscreen(w(9)));
}

#[test]
fn fullscreen_window_on_hidden_workspace_is_hidden() {
    let mut wm = wm();
    manage_all(&mut wm, [1]);
    wm.request_fullscreen(w(1), true);
    apply(&mut wm, "view 2");
    let a = wm.arrange();
    assert_eq!(a.windows.len(), 0);
    assert_eq!(a.hidden, [w(1)]);
}

#[test]
fn zero_border_width_means_no_borders() {
    let mut config = Config::defaults();
    config.border.width = 0;
    let mut wm = Wm::new(config);
    wm.add_output(O0, "DP-1", SCREEN);
    manage_all(&mut wm, [1]);
    let a = wm.arrange();
    assert_eq!(a.windows[0].border, None);
    assert_eq!(a.windows[0].rect, AREA);
}

#[test]
fn usable_rect_and_bar_position() {
    let mut config = Config::defaults();
    config.bar.position = crate::config::BarPosition::Bottom;
    let mut wm = Wm::new(config);
    wm.add_output(O0, "DP-1", SCREEN);
    wm.set_usable_rect(O0, Rect::new(0, 30, 1920, 1050));
    manage_all(&mut wm, [1]);
    let a = wm.arrange();
    assert_eq!(a.bar, Some((O0, Rect::new(0, 1056, 1920, 24))));
    assert_eq!(wm.tiling_area(S0), Some(Rect::new(0, 30, 1920, 1026)));
    assert_eq!(rect_of(&a, 1), Some(Rect::new(1, 31, 1918, 1024)));
}

#[test]
fn bar_goes_to_configured_output() {
    let mut config = Config::defaults();
    config.bar.output = Some("DP-2".into());
    let mut wm = Wm::new(config);
    wm.add_output(O0, "DP-1", SCREEN);
    wm.add_output(O1, "DP-2", Rect::new(1920, 0, 1920, 1080));
    assert_eq!(wm.bar_output(), Some(O1));
    let a = wm.arrange();
    assert_eq!(a.bar, Some((O1, Rect::new(1920, 0, 1920, 24))));
    assert_eq!(wm.tiling_area(S0), Some(SCREEN));
    // An unknown output falls back to the first screen.
    let mut config = Config::defaults();
    config.bar.output = Some("HDMI-9".into());
    let mut wm = Wm::new(config);
    wm.add_output(O1, "DP-2", SCREEN);
    wm.add_output(O0, "DP-1", SCREEN);
    assert_eq!(wm.bar_output(), Some(O0));
}

#[test]
fn copies_visible_on_two_screens_are_placed_once() {
    let mut wm = wm2();
    manage_all(&mut wm, [1]);
    apply(&mut wm, "copy 2");
    // Workspace 2 is on S1; the current screen S0 wins.
    let a = wm.arrange();
    assert_eq!(a.windows.len(), 1);
    assert_eq!(a.windows[0].screen, S0);
    apply(&mut wm, "focus-screen next");
    let a = wm.arrange();
    assert_eq!(a.windows.len(), 1);
    assert_eq!(a.windows[0].screen, S1);
    // Neither screen current: the first in screen order wins.
    wm.add_output(OutputId(5), "DP-3", Rect::new(3840, 0, 1920, 1080));
    wm.stack_set.focus_screen(ScreenId(5));
    let a = wm.arrange();
    assert_eq!(a.windows[0].screen, S0);
}

#[test]
fn bar_model_states_and_order() {
    let mut wm = wm2();
    manage_all(&mut wm, [1]);
    apply(&mut wm, "shift 5");
    wm.apply(Action::Prompt(crate::prompt::PromptKind::ViewWorkspace));
    for c in "web".chars() {
        wm.prompt_key(crate::prompt::PromptKey::Char(c));
    }
    wm.prompt_key(crate::prompt::PromptKey::Return);
    let model = wm.bar_model();
    let tags: Vec<&str> = model.workspaces.iter().map(|(t, _)| t.as_str()).collect();
    assert_eq!(tags, ["1", "2", "3", "4", "5", "6", "7", "8", "9", "web"]);
    let state = |tag: &str| {
        model
            .workspaces
            .iter()
            .find(|(t, _)| t == tag)
            .map(|(_, s)| *s)
    };
    assert_eq!(state("web"), Some(WorkspaceState::Current));
    assert_eq!(state("2"), Some(WorkspaceState::Visible));
    assert_eq!(state("5"), Some(WorkspaceState::HiddenNonEmpty));
    assert_eq!(state("1"), Some(WorkspaceState::HiddenEmpty));
}

#[test]
fn bar_title_is_shortened() {
    let mut wm = wm();
    let long: String = "x".repeat(150);
    wm.manage(w(1), info("app", &long));
    let title = wm.bar_model().title;
    assert_eq!(title.chars().count(), 100);
    assert!(title.ends_with("..."));
    wm.update_info(w(1), info("app", "short"));
    assert_eq!(wm.bar_model().title, "short");
    assert_eq!(super::super::view::shorten("abc", 3), "...");
    assert_eq!(super::super::view::shorten("ab", 3), "ab");
}
