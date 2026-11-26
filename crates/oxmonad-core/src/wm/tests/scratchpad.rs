use super::*;
use crate::stack_set::SCRATCHPAD_TAG;

fn scratch() -> WindowInfo {
    info("scratchpad", "term")
}

#[test]
fn toggle_spawns_when_missing() {
    let mut wm = wm();
    assert_eq!(
        apply(&mut wm, "toggle-scratchpad term"),
        [Effect::Spawn("alacritty --class scratchpad".into())]
    );
    // Unknown scratchpads do nothing.
    assert_eq!(apply(&mut wm, "toggle-scratchpad nope").len(), 0);
}

#[test]
fn new_scratchpad_window_floats_at_its_rect() {
    let mut wm = wm();
    manage_all(&mut wm, [1]);
    wm.manage(w(2), scratch());
    assert!(wm.stack_set().is_floating(w(2)));
    let p = wm.arrange().placement(w(2)).cloned().expect("placed");
    assert_eq!(p.layer, Layer::Floating);
    // [0.1, 0.1, 0.8, 0.8] of 1920x1080, minus the 1px border.
    assert_eq!(p.rect.outset(1), Rect::new(192, 108, 1536, 864));
}

#[test]
fn toggle_hides_and_summons() {
    let mut wm = wm();
    manage_all(&mut wm, [1]);
    wm.manage(w(2), scratch());
    // Visible on the current workspace: hide it on NSP.
    assert_eq!(apply(&mut wm, "toggle-scratchpad term").len(), 0);
    assert_eq!(windows(&wm), [w(1)]);
    assert_eq!(
        wm.stack_set()
            .workspace(SCRATCHPAD_TAG)
            .map(Workspace::windows),
        Some(vec![w(2)])
    );
    assert!(wm.arrange().hidden.contains(&w(2)));
    // NSP is not in the bar.
    assert!(
        !wm.bar_model(O0)
            .workspaces
            .iter()
            .any(|(t, _)| t == SCRATCHPAD_TAG)
    );
    // Summon it onto another workspace.
    apply(&mut wm, "view 3");
    assert_eq!(apply(&mut wm, "toggle-scratchpad term").len(), 0);
    assert_eq!(windows(&wm), [w(2)]);
    assert_eq!(wm.stack_set().peek(), Some(w(2)));
    assert!(wm.stack_set().is_floating(w(2)));
}

#[test]
fn summon_from_another_visible_workspace() {
    let mut wm = wm2();
    wm.manage(w(1), scratch());
    apply(&mut wm, "focus-screen next");
    apply(&mut wm, "toggle-scratchpad term");
    assert_eq!(windows(&wm), [w(1)]);
    assert_eq!(wm.arrange().placement(w(1)).map(|p| p.screen), Some(S1));
}

#[test]
fn summoned_sunk_scratchpad_floats_again() {
    let mut wm = wm();
    wm.manage(w(1), scratch());
    apply(&mut wm, "sink");
    apply(&mut wm, "shift 4");
    apply(&mut wm, "toggle-scratchpad term");
    assert!(wm.stack_set().is_floating(w(1)));
    assert_eq!(windows(&wm), [w(1)]);
}

#[test]
fn toggle_ws_skips_scratchpad_workspace() {
    let mut wm = wm();
    wm.manage(w(1), scratch());
    apply(&mut wm, "view 2");
    apply(&mut wm, "view 1");
    apply(&mut wm, "toggle-scratchpad term");
    // NSP is now the most recently hidden workspace; toggle-ws skips it.
    apply(&mut wm, "toggle-ws");
    assert_eq!(wm.stack_set().current_tag(), "2");
}

/// A `Wm` whose `gui` scratchpad holds two applications.
fn wm_gui() -> Wm {
    let config = Config::from_toml(
        r#"
[[scratchpads]]
name = "gui"
rect = [0.1, 0.1, 0.8, 0.8]
apps = [
  { command = "mycelium-gui", app_id = "mycelium" },
  { command = "rhyolite-gui", app_id = "rhyolite" },
]
"#,
    )
    .expect("config loads");
    let mut wm = Wm::new(config);
    wm.add_output(O0, "DP-1", SCREEN);
    wm
}

fn strips(wm: &Wm) -> Vec<TabStrip> {
    wm.arrange()
        .screens
        .into_iter()
        .flat_map(|s| s.scratchpad_strips)
        .collect()
}

#[test]
fn group_toggle_spawns_every_missing_app() {
    let mut wm = wm_gui();
    assert_eq!(
        apply(&mut wm, "toggle-scratchpad gui"),
        [
            Effect::Spawn("mycelium-gui".into()),
            Effect::Spawn("rhyolite-gui".into())
        ]
    );
    // With one app running, summoning spawns only the other.
    wm.manage(w(1), info("mycelium", "Mycelium"));
    apply(&mut wm, "toggle-scratchpad gui");
    apply(&mut wm, "view 2");
    assert_eq!(
        apply(&mut wm, "toggle-scratchpad gui"),
        [Effect::Spawn("rhyolite-gui".into())]
    );
    assert_eq!(windows(&wm), [w(1)]);
}

#[test]
fn group_windows_share_the_rect_as_tabs() {
    let mut wm = wm_gui();
    manage_all(&mut wm, [1]);
    wm.manage(w(2), info("mycelium", "Mycelium"));
    // A single window has no tabs.
    assert_eq!(strips(&wm), []);
    wm.manage(w(3), info("rhyolite", "Rhyolite"));
    let arrangement = wm.arrange();
    // The focused one fills the rect below the strip, the other is hidden.
    assert_eq!(
        rect_of(&arrangement, 3).map(|r| r.outset(1)),
        Some(Rect::new(192, 128, 1536, 844))
    );
    assert!(arrangement.hidden.contains(&w(2)));
    let strip = &strips(&wm)[0];
    assert_eq!(strip.rect, Rect::new(192, 108, 1536, 20));
    let tabs: Vec<_> = strip
        .tabs
        .iter()
        .map(|t| (t.window, t.title.as_str(), t.focused))
        .collect();
    assert_eq!(tabs, [(w(3), "Rhyolite", true), (w(2), "Mycelium", false)]);
    // Clicking a tab selects it.
    wm.focus_window(w(2));
    assert!(wm.arrange().hidden.contains(&w(3)));
    assert!(wm.arrange().placement(w(2)).is_some());
}

#[test]
fn group_keeps_its_tab_when_focus_leaves() {
    let mut wm = wm_gui();
    manage_all(&mut wm, [1]);
    wm.manage(w(2), info("mycelium", "Mycelium"));
    wm.manage(w(3), info("rhyolite", "Rhyolite"));
    wm.focus_window(w(2));
    wm.focus_window(w(1));
    let arrangement = wm.arrange();
    assert!(arrangement.placement(w(2)).is_some());
    assert!(arrangement.hidden.contains(&w(3)));
    // Hiding and summoning keeps the selected tab, too.
    apply(&mut wm, "toggle-scratchpad gui");
    assert_eq!(windows(&wm), [w(1)]);
    apply(&mut wm, "toggle-scratchpad gui");
    assert_eq!(wm.stack_set().peek(), Some(w(2)));
    assert!(wm.arrange().hidden.contains(&w(3)));
}

#[test]
fn focus_next_and_prev_cycle_the_tabs() {
    let mut wm = wm_gui();
    manage_all(&mut wm, [1, 2]);
    wm.manage(w(3), info("mycelium", "Mycelium"));
    wm.manage(w(4), info("rhyolite", "Rhyolite"));
    wm.manage(w(5), info("mycelium", "Mycelium 2"));
    // Tab order is stack order: 5, 4, 3. Focus stays among the tabs.
    let mut seen = Vec::new();
    for _ in 0..4 {
        apply(&mut wm, "focus-next");
        seen.push(wm.stack_set().peek());
    }
    assert_eq!(seen, [Some(w(4)), Some(w(3)), Some(w(5)), Some(w(4))]);
    apply(&mut wm, "focus-prev");
    apply(&mut wm, "focus-prev");
    assert_eq!(wm.stack_set().peek(), Some(w(3)));
    // Outside the scratchpad, focus moves as usual.
    wm.focus_window(w(2));
    apply(&mut wm, "focus-next");
    assert_eq!(wm.stack_set().peek(), Some(w(1)));
}

#[test]
fn a_lone_scratchpad_window_has_no_tabs() {
    let mut wm = wm_gui();
    manage_all(&mut wm, [1]);
    wm.manage(w(2), info("mycelium", "Mycelium"));
    wm.manage(w(3), info("rhyolite", "Rhyolite"));
    assert_eq!(strips(&wm).len(), 1);
    // Once one tab is closed, the other is a plain float without a strip.
    wm.unmanage(w(3));
    assert_eq!(strips(&wm), []);
    assert_eq!(
        rect_of(&wm.arrange(), 2).map(|r| r.outset(1)),
        Some(Rect::new(192, 108, 1536, 864))
    );
    // And focus-next leaves it as for any window.
    wm.focus_window(w(2));
    apply(&mut wm, "focus-next");
    assert_eq!(wm.stack_set().peek(), Some(w(1)));
}
