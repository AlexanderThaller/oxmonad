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
        !wm.bar_model()
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
