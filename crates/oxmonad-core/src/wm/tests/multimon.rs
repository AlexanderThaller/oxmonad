use super::*;

fn screen_of(wm: &Wm, tag: &str) -> Option<ScreenId> {
    wm.stack_set().screen_of_tag(tag)
}

fn current_screen(wm: &Wm) -> Option<ScreenId> {
    wm.stack_set().current_screen().map(|s| s.id)
}

#[test]
fn second_screen_shows_next_workspace() {
    let wm = wm2();
    assert_eq!(screen_of(&wm, "1"), Some(S0));
    assert_eq!(screen_of(&wm, "2"), Some(S1));
    // Each screen tiles its own workspace.
    let a = wm.arrange();
    assert_eq!(a.screens.len(), 2);
    assert!(a.screens[1].bar.is_none());
}

#[test]
fn windows_tile_on_their_screen() {
    let mut wm = wm2();
    manage_all(&mut wm, [1]);
    apply(&mut wm, "focus-screen next");
    manage_all(&mut wm, [2]);
    let a = wm.arrange();
    assert_eq!(rect_of(&a, 1), Some(AREA.inset(1)));
    // No bar on the second screen.
    assert_eq!(rect_of(&a, 2), Some(Rect::new(1921, 1, 1918, 1078)));
    assert_eq!(a.focus, Focus::Window(w(2)));
}

#[test]
fn view_uses_greedy_view() {
    let mut wm = wm2();
    apply(&mut wm, "view 2");
    assert_eq!(screen_of(&wm, "2"), Some(S0));
    assert_eq!(screen_of(&wm, "1"), Some(S1));
    assert_eq!(current_screen(&wm), Some(S0));
}

#[test]
fn focus_screen_cycles_physically() {
    let mut wm = wm2();
    apply(&mut wm, "focus-screen next");
    assert_eq!(current_screen(&wm), Some(S1));
    apply(&mut wm, "focus-screen next");
    assert_eq!(current_screen(&wm), Some(S0));
    apply(&mut wm, "focus-screen prev");
    assert_eq!(current_screen(&wm), Some(S1));
}

#[test]
fn shift_screen_keeps_focus_follow_variant_follows() {
    let mut wm = wm2();
    manage_all(&mut wm, [1, 2]);
    apply(&mut wm, "shift-screen next");
    assert_eq!(current_screen(&wm), Some(S0));
    assert_eq!(windows(&wm), [w(1)]);
    assert_eq!(
        wm.stack_set().workspace("2").map(Workspace::windows),
        Some(vec![w(2)])
    );
    // With two screens "prev" is S1 as well; this time focus follows.
    apply(&mut wm, "shift-screen-follow prev");
    assert_eq!(current_screen(&wm), Some(S1));
    assert_eq!(wm.stack_set().peek(), Some(w(1)));
    assert_eq!(windows(&wm), [w(1), w(2)]);
    assert!(
        wm.stack_set()
            .workspace("1")
            .is_some_and(Workspace::is_empty)
    );
    assert_eq!(wm.arrange().focus, Focus::Window(w(1)));
}

#[test]
fn swap_next_screen_exchanges_workspaces() {
    let mut wm = wm2();
    manage_all(&mut wm, [1]);
    apply(&mut wm, "swap-next-screen");
    assert_eq!(screen_of(&wm, "2"), Some(S0));
    assert_eq!(screen_of(&wm, "1"), Some(S1));
    assert_eq!(wm.arrange().placement(w(1)).map(|p| p.screen), Some(S1));
}

#[test]
fn clicking_a_window_on_another_screen_focuses_that_screen() {
    let mut wm = wm2();
    manage_all(&mut wm, [1]);
    apply(&mut wm, "focus-screen next");
    manage_all(&mut wm, [2]);
    apply(&mut wm, "focus-screen next");
    assert_eq!(current_screen(&wm), Some(S0));
    wm.focus_window(w(2));
    assert_eq!(current_screen(&wm), Some(S1));
    assert_eq!(wm.arrange().focus, Focus::Window(w(2)));
}

#[test]
fn dragging_a_float_onto_another_screen_moves_it_there() {
    let mut wm = wm2();
    manage_all(&mut wm, [1]);
    wm.float_at(w(1), Rect::new(100, 100, 400, 300));
    wm.set_float_rect(w(1), Rect::new(2500, 200, 400, 300));
    assert_eq!(screen_of(&wm, "2"), Some(S1));
    assert_eq!(
        wm.stack_set().workspace("2").map(Workspace::windows),
        Some(vec![w(1)])
    );
    assert_eq!(current_screen(&wm), Some(S1));
    let p = wm.arrange().placement(w(1)).cloned().expect("placed");
    assert_eq!((p.screen, p.rect), (S1, Rect::new(2500, 200, 400, 300)));
}

#[test]
fn unplugging_a_screen_keeps_its_windows() {
    let mut wm = wm2();
    apply(&mut wm, "focus-screen next");
    manage_all(&mut wm, [1]);
    wm.remove_screen(S1);
    assert_eq!(current_screen(&wm), Some(S0));
    assert!(wm.arrange().hidden.contains(&w(1)));
    // Replugging brings the most recently hidden workspace back.
    wm.add_screen(S1, "DP-2", Rect::new(1920, 0, 1920, 1080));
    assert_eq!(screen_of(&wm, "2"), Some(S1));
    assert_eq!(wm.arrange().placement(w(1)).map(|p| p.screen), Some(S1));
}

#[test]
fn removing_workspace_on_second_screen() {
    let mut wm = wm2();
    apply(&mut wm, "focus-screen next");
    manage_all(&mut wm, [1]);
    apply(&mut wm, "view 7");
    manage_all(&mut wm, [2]);
    apply(&mut wm, "remove-workspace");
    // S1 falls back to "2" (most recently hidden); S0 is untouched.
    assert_eq!(screen_of(&wm, "1"), Some(S0));
    assert_eq!(screen_of(&wm, "2"), Some(S1));
    assert_eq!(current_screen(&wm), Some(S1));
    assert_eq!(windows(&wm), [w(2), w(1)]);
    assert!(!wm.stack_set().has_tag("7"));
}

#[test]
fn removing_workspace_with_nothing_hidden_is_refused() {
    let mut config = Config::defaults();
    config.workspaces = vec!["a".into(), "b".into()];
    let mut wm = Wm::new(config);
    wm.add_screen(S0, "DP-1", SCREEN);
    wm.add_screen(S1, "DP-2", Rect::new(1920, 0, 1920, 1080));
    manage_all(&mut wm, [1]);
    apply(&mut wm, "remove-workspace");
    assert_eq!(screen_of(&wm, "a"), Some(S0));
    assert_eq!(screen_of(&wm, "b"), Some(S1));
    assert_eq!(windows(&wm), [w(1)]);
}

#[test]
fn more_screens_than_workspaces_creates_workspaces() {
    let mut config = Config::defaults();
    config.workspaces = vec!["a".into()];
    let mut wm = Wm::new(config);
    wm.add_screen(S0, "DP-1", SCREEN);
    wm.add_screen(S1, "DP-2", Rect::new(1920, 0, 1920, 1080));
    assert_eq!(screen_of(&wm, "1"), Some(S1));
    let tags: Vec<String> = wm
        .bar_model()
        .workspaces
        .into_iter()
        .map(|(t, _)| t)
        .collect();
    assert_eq!(tags, ["a", "1"]);
}

#[test]
fn pointer_on_other_screen_makes_it_current() {
    let mut wm = wm2();
    manage_all(&mut wm, [1]);
    // An empty second screen still becomes current.
    wm.focus_screen_at(2500, 500);
    assert_eq!(current_screen(&wm), Some(S1));
    assert_eq!(wm.arrange().focus, Focus::None);
    // Back on the first screen, its focused window gets focus again.
    wm.focus_screen_at(100, 100);
    assert_eq!(current_screen(&wm), Some(S0));
    assert_eq!(wm.arrange().focus, Focus::Window(w(1)));
    // Off every screen: nothing changes.
    wm.focus_screen_at(-50, -50);
    assert_eq!(current_screen(&wm), Some(S0));
}

#[test]
fn pointer_does_not_switch_screens_while_prompting() {
    let mut wm = wm2();
    apply(&mut wm, "prompt view");
    wm.focus_screen_at(2500, 500);
    assert_eq!(current_screen(&wm), Some(S0));
}
