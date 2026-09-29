use std::collections::BTreeSet;

use super::*;
use crate::{
    action::Direction,
    geometry::Ratio,
};

const S0: ScreenId = ScreenId(0);
const S1: ScreenId = ScreenId(1);
const S2: ScreenId = ScreenId(2);

fn w(n: u64) -> WindowId {
    WindowId(n)
}

fn tags(n: usize) -> Vec<String> {
    (1..=n).map(|i| i.to_string()).collect()
}

fn new_set(n: usize) -> StackSet {
    StackSet::new(&tags(n), LayoutRing::xmonad_default(20))
}

/// A stack set with 9 workspaces and one 1920x1080 screen.
fn one_screen() -> StackSet {
    let mut s = new_set(9);
    s.add_screen(S0, "DP-1", Rect::new(0, 0, 1920, 1080));
    s
}

/// Three screens: S0 left, S1 right, S2 below S0.
fn three_screens() -> StackSet {
    let mut s = new_set(9);
    s.add_screen(S0, "DP-1", Rect::new(0, 0, 1920, 1080));
    s.add_screen(S1, "DP-2", Rect::new(1920, 0, 1920, 1080));
    s.add_screen(S2, "HDMI-1", Rect::new(0, 1080, 1920, 1080));
    s
}

fn screen_tags(s: &StackSet) -> Vec<(u64, String)> {
    s.screens()
        .iter()
        .map(|sc| (sc.id.0, sc.workspace.tag.clone()))
        .collect()
}

fn hidden_tags(s: &StackSet) -> Vec<&str> {
    s.hidden().iter().map(|w| w.tag.as_str()).collect()
}

fn full_rr() -> RationalRect {
    RationalRect::new(
        Ratio::new(1, 4),
        Ratio::new(1, 4),
        Ratio::new(1, 2),
        Ratio::new(1, 2),
    )
}

/// Checks the structural invariants of a stack set.
fn check_invariants(s: &StackSet) {
    let all: Vec<&Workspace> = s.workspaces().collect();
    let tags: BTreeSet<&str> = all.iter().map(|w| w.tag.as_str()).collect();
    assert_eq!(tags.len(), all.len(), "workspace tags are unique");
    for ws in &all {
        let windows = ws.windows();
        let unique: BTreeSet<_> = windows.iter().collect();
        assert_eq!(unique.len(), windows.len(), "no duplicates in {}", ws.tag);
    }
    let members = s.all_windows();
    for window in s.floating().keys() {
        assert!(members.contains(window), "floating {window:?} is managed");
    }
    let ids: Vec<ScreenId> = s.screens().iter().map(|sc| sc.id).collect();
    let mut sorted = ids.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(ids, sorted, "screens sorted and unique");
    if s.is_headless() {
        assert!(s.current_screen().is_none());
    } else {
        assert!(s.current_screen().is_some());
    }
}

#[test]
fn new_is_headless_with_first_tag_current() {
    let s = new_set(3);
    assert!(s.is_headless());
    assert_eq!(s.screens().len(), 0);
    assert_eq!(s.current_tag(), "1");
    assert_eq!(hidden_tags(&s), ["2", "3"]);
    let empty = StackSet::new(&[], LayoutRing::xmonad_default(20));
    assert_eq!(empty.current_tag(), "1");
}

#[test]
fn first_screen_adopts_current_workspace() {
    let mut s = new_set(3);
    s.insert_up(w(1));
    s.add_screen(S0, "DP-1", Rect::new(0, 0, 100, 100));
    assert!(!s.is_headless());
    assert_eq!(screen_tags(&s), [(0, "1".into())]);
    assert_eq!(s.peek(), Some(w(1)));
}

#[test]
fn new_screens_take_most_recently_hidden_then_fresh_tags() {
    let mut s = new_set(2);
    s.add_screen(S0, "a", Rect::new(0, 0, 100, 100));
    s.add_screen(S1, "b", Rect::new(100, 0, 100, 100));
    s.add_screen(S2, "c", Rect::new(200, 0, 100, 100));
    assert_eq!(
        screen_tags(&s),
        [(0, "1".into()), (1, "2".into()), (2, "3".into())]
    );
    // The scratchpad workspace is never put on a screen.
    s.remove_screen(S2);
    s.add_workspace(SCRATCHPAD_TAG);
    s.add_screen(ScreenId(7), "d", Rect::new(300, 0, 100, 100));
    assert_eq!(
        s.screen(ScreenId(7)).map(|x| x.workspace.tag.as_str()),
        Some("3")
    );
    s.add_screen(ScreenId(8), "e", Rect::new(400, 0, 100, 100));
    assert_eq!(
        s.screen(ScreenId(8)).map(|x| x.workspace.tag.as_str()),
        Some("4")
    );
    check_invariants(&s);
}

#[test]
fn removing_screens_hides_their_workspaces() {
    let mut s = three_screens();
    s.focus_screen(S1);
    s.insert_up(w(1));
    s.remove_screen(S1);
    // S1's workspace "2" is now the most recently hidden.
    assert_eq!(hidden_tags(&s)[0], "2");
    assert!(s.member(w(1)));
    // The first screen in physical order (S0, top-left) becomes current.
    assert_eq!(s.current_screen().map(|x| x.id), Some(S0));
    s.remove_screen(S0);
    assert_eq!(s.current_screen().map(|x| x.id), Some(S2));
    s.remove_screen(S2);
    assert!(s.is_headless());
    assert_eq!(s.current_tag(), "3");
    s.remove_screen(S2);
    assert!(s.is_headless());
    check_invariants(&s);
}

#[test]
fn removing_a_non_current_screen_keeps_focus() {
    let mut s = three_screens();
    s.focus_screen(S2);
    s.remove_screen(S0);
    assert_eq!(s.current_screen().map(|x| x.id), Some(S2));
}

#[test]
fn add_screen_twice_updates_geometry() {
    let mut s = one_screen();
    s.add_screen(S0, "DP-1", Rect::new(0, 0, 800, 600));
    assert_eq!(s.screens().len(), 1);
    assert_eq!(s.screens()[0].rect, Rect::new(0, 0, 800, 600));
    s.set_usable_rect(S0, Rect::new(0, 30, 800, 570));
    assert_eq!(s.screens()[0].usable_rect, Rect::new(0, 30, 800, 570));
    s.set_screen_rect(S0, Rect::new(0, 0, 1024, 768));
    assert_eq!(s.screens()[0].usable_rect, Rect::new(0, 0, 1024, 768));
}

#[test]
fn view_hidden_swaps_into_current_screen_and_records_history() {
    let mut s = one_screen();
    s.view("3");
    assert_eq!(s.current_tag(), "3");
    assert_eq!(hidden_tags(&s)[0], "1");
    s.view("5");
    assert_eq!(hidden_tags(&s)[..2], ["3", "1"]);
    s.view("nope");
    assert_eq!(s.current_tag(), "5");
}

#[test]
fn view_visible_focuses_its_screen_greedy_view_swaps() {
    let mut s = three_screens();
    assert_eq!(s.current_tag(), "1");
    s.view("2");
    assert_eq!(s.current_screen().map(|x| x.id), Some(S1));
    assert_eq!(screen_tags(&s)[1], (1, "2".into()));

    let mut s = three_screens();
    s.greedy_view("2");
    assert_eq!(s.current_screen().map(|x| x.id), Some(S0));
    assert_eq!(
        screen_tags(&s),
        [(0, "2".into()), (1, "1".into()), (2, "3".into())]
    );
    // Greedy view of a hidden workspace is a plain view.
    s.greedy_view("7");
    assert_eq!(screen_tags(&s)[0], (0, "7".into()));
    assert_eq!(hidden_tags(&s)[0], "2");
    check_invariants(&s);
}

#[test]
fn physical_order_is_top_to_bottom_then_left_to_right() {
    let s = three_screens();
    assert_eq!(s.physical_order(), [S0, S1, S2]);
    let mut s = new_set(9);
    s.add_screen(S0, "a", Rect::new(1920, 0, 1920, 1080));
    s.add_screen(S1, "b", Rect::new(0, 0, 1920, 1080));
    s.add_screen(S2, "c", Rect::new(0, -1080, 1920, 1080));
    assert_eq!(s.physical_order(), [S2, S1, S0]);
    s.focus_screen(S1);
    assert_eq!(s.neighbour(Direction::Next), Some(S0));
    assert_eq!(s.neighbour(Direction::Prev), Some(S2));
    s.focus_screen(S0);
    assert_eq!(s.neighbour(Direction::Next), Some(S2));
}

#[test]
fn swap_next_screen_uses_screen_id_order() {
    let mut s = three_screens();
    s.focus_screen(S2);
    s.swap_next_screen();
    assert_eq!(
        screen_tags(&s),
        [(0, "3".into()), (1, "2".into()), (2, "1".into())]
    );
    assert_eq!(s.current_screen().map(|x| x.id), Some(S2));
    let mut single = one_screen();
    single.swap_next_screen();
    assert_eq!(single.current_tag(), "1");
}

#[test]
fn insert_and_stack_operations() {
    let mut s = one_screen();
    for i in 1..=3 {
        s.insert_up(w(i));
    }
    // Each new window goes above the focus: 3 2 1, focus 3.
    assert_eq!(s.current_workspace().windows(), [w(3), w(2), w(1)]);
    assert_eq!(s.peek(), Some(w(3)));
    s.insert_up(w(3));
    assert_eq!(s.current_workspace().windows().len(), 3);
    s.focus_down();
    assert_eq!(s.peek(), Some(w(2)));
    s.swap_master();
    assert_eq!(s.current_workspace().windows(), [w(2), w(3), w(1)]);
    s.swap_down();
    assert_eq!(s.current_workspace().windows(), [w(3), w(2), w(1)]);
    s.swap_up();
    s.focus_up();
    assert_eq!(s.peek(), Some(w(1)));
    s.focus_master();
    assert_eq!(s.peek(), Some(w(2)));
    s.focus_down();
    s.shift_master();
    assert_eq!(s.current_workspace().windows(), [w(3), w(2), w(1)]);
}

#[test]
fn delete_removes_from_all_workspaces_and_sinks() {
    let mut s = one_screen();
    s.insert_up(w(1));
    s.insert_up(w(2));
    s.copy("4");
    s.float(w(2), full_rr());
    s.delete(w(2));
    assert!(!s.member(w(2)));
    assert!(!s.is_floating(w(2)));
    assert!(s.workspace("4").is_some_and(Workspace::is_empty));
    assert_eq!(s.peek(), Some(w(1)));
    check_invariants(&s);
}

#[test]
fn shift_moves_focused_window() {
    let mut s = one_screen();
    s.insert_up(w(1));
    s.insert_up(w(2));
    s.shift("3");
    assert_eq!(s.current_workspace().windows(), [w(1)]);
    assert_eq!(s.workspace("3").map(Workspace::windows), Some(vec![w(2)]));
    // Unknown tags and the current tag are no-ops.
    s.shift("missing");
    s.shift("1");
    assert_eq!(s.current_workspace().windows(), [w(1)]);
    s.shift_win("1", w(2));
    assert_eq!(s.current_workspace().windows(), [w(2), w(1)]);
}

#[test]
fn copy_and_shift_of_copies() {
    let mut s = one_screen();
    s.insert_up(w(1));
    s.copy("2");
    s.copy("2");
    assert_eq!(s.workspace("2").map(Workspace::windows), Some(vec![w(1)]));
    assert_eq!(s.current_workspace().windows(), [w(1)]);
    // Shifting a copied window moves this copy; the target still gets it.
    s.shift("3");
    assert!(s.current_workspace().stack.is_none());
    assert_eq!(s.workspace("3").map(Workspace::windows), Some(vec![w(1)]));
    assert_eq!(s.workspace("2").map(Workspace::windows), Some(vec![w(1)]));
    check_invariants(&s);
}

#[test]
fn focus_window_switches_screens_and_workspaces() {
    let mut s = three_screens();
    s.insert_up(w(1));
    s.focus_screen(S1);
    s.insert_up(w(2));
    s.insert_up(w(3));
    s.focus_screen(S0);
    s.focus_window(w(2));
    assert_eq!(s.current_screen().map(|x| x.id), Some(S1));
    assert_eq!(s.peek(), Some(w(2)));
    // A window on a hidden workspace is brought to the current screen.
    s.shift("7");
    s.focus_window(w(2));
    assert_eq!(s.current_tag(), "7");
    assert_eq!(s.current_screen().map(|x| x.id), Some(S1));
    s.focus_window(w(99));
    assert_eq!(s.current_tag(), "7");
}

#[test]
fn float_and_sink() {
    let mut s = one_screen();
    s.insert_up(w(1));
    s.float(w(1), full_rr());
    assert!(s.is_floating(w(1)));
    s.sink(w(1));
    assert!(!s.is_floating(w(1)));
}

#[test]
fn add_and_rename_workspaces() {
    let mut s = one_screen();
    assert!(s.add_workspace("web"));
    assert!(!s.add_workspace("web"));
    assert!(!s.add_workspace(""));
    assert_eq!(hidden_tags(&s)[0], "web");
    assert!(s.rename_current("mail"));
    assert_eq!(s.current_tag(), "mail");
    assert!(!s.rename_current(""));
    assert!(!s.rename_current("mail"));
    check_invariants(&s);
}

#[test]
fn rename_onto_hidden_workspace_merges_it() {
    let mut s = one_screen();
    s.insert_up(w(1));
    s.shift("2");
    s.insert_up(w(3));
    assert!(s.rename_current("2"));
    assert_eq!(s.current_tag(), "2");
    // The hidden "2"'s windows come first and take the focus.
    assert_eq!(s.current_workspace().windows(), [w(1), w(3)]);
    assert_eq!(s.peek(), Some(w(1)));
    assert!(!hidden_tags(&s).contains(&"2"));
    check_invariants(&s);
}

#[test]
fn rename_onto_visible_workspace_is_refused() {
    let mut s = three_screens();
    assert!(!s.rename_current("2"));
    assert_eq!(s.current_tag(), "1");
}

#[test]
fn remove_workspace_merges_into_most_recently_hidden() {
    let mut s = one_screen();
    s.view("4");
    s.insert_up(w(1));
    s.view("5");
    s.insert_up(w(2));
    s.insert_up(w(3));
    // History: 4, 1, 2, ...
    assert!(s.remove_current_workspace());
    assert_eq!(s.current_tag(), "4");
    assert!(!s.has_tag("5"));
    assert_eq!(s.current_workspace().windows(), [w(3), w(2), w(1)]);
    assert_eq!(s.peek(), Some(w(3)));
    check_invariants(&s);
}

#[test]
fn remove_workspace_needs_a_hidden_non_scratchpad_workspace() {
    let mut s = new_set(2);
    s.add_screen(S0, "a", Rect::new(0, 0, 100, 100));
    s.add_screen(S1, "b", Rect::new(100, 0, 100, 100));
    s.add_workspace(SCRATCHPAD_TAG);
    assert!(!s.remove_current_workspace());
    assert_eq!(screen_tags(&s), [(0, "1".into()), (1, "2".into())]);
}

#[test]
fn remove_workspace_on_second_monitor_leaves_other_screens_alone() {
    let mut s = three_screens();
    s.focus_screen(S1);
    s.insert_up(w(1));
    s.view("6");
    s.insert_up(w(2));
    // S1 shows 6, history starts with 2.
    assert!(s.remove_current_workspace());
    assert_eq!(
        screen_tags(&s),
        [(0, "1".into()), (1, "2".into()), (2, "3".into())]
    );
    assert_eq!(s.current_screen().map(|x| x.id), Some(S1));
    assert_eq!(s.current_workspace().windows(), [w(2), w(1)]);
    check_invariants(&s);
}

#[test]
fn remove_workspace_never_steals_a_visible_workspace() {
    // Only visible workspaces besides the current one and a single hidden.
    let mut s = new_set(3);
    s.add_screen(S0, "a", Rect::new(0, 0, 100, 100));
    s.add_screen(S1, "b", Rect::new(100, 0, 100, 100));
    s.focus_screen(S1);
    s.insert_up(w(5));
    assert!(s.remove_current_workspace());
    assert_eq!(screen_tags(&s), [(0, "1".into()), (1, "3".into())]);
    assert_eq!(s.current_workspace().windows(), [w(5)]);
    // Now nothing is hidden: the next removal is refused.
    assert!(!s.remove_current_workspace());
    assert_eq!(screen_tags(&s), [(0, "1".into()), (1, "3".into())]);
    check_invariants(&s);
}

#[test]
fn remove_workspace_with_copies_does_not_duplicate() {
    let mut s = one_screen();
    s.insert_up(w(1));
    s.copy("2");
    s.view("2");
    s.insert_up(w(2));
    s.view("1");
    s.view("2");
    // Current "2" holds 2 and 1; history starts with "1" holding 1.
    assert!(s.remove_current_workspace());
    assert_eq!(s.current_tag(), "1");
    assert_eq!(s.current_workspace().windows(), [w(2), w(1)]);
    check_invariants(&s);
}

#[test]
fn remove_workspace_when_headless() {
    let mut s = new_set(3);
    s.insert_up(w(1));
    assert!(s.remove_current_workspace());
    assert_eq!(s.current_tag(), "2");
    assert_eq!(s.current_workspace().windows(), [w(1)]);
    check_invariants(&s);
}

#[test]
fn toggle_ws_skips_scratchpad() {
    let mut s = one_screen();
    s.view("3");
    s.add_workspace(SCRATCHPAD_TAG);
    s.toggle_ws(&[]);
    assert_eq!(s.current_tag(), "1");
    s.toggle_ws(&[]);
    assert_eq!(s.current_tag(), "3");
    s.toggle_ws(&["1"]);
    assert_eq!(s.current_tag(), "2");
}

#[test]
fn reset_layout_restores_template() {
    let mut s = one_screen();
    s.current_workspace_mut().layout.next();
    assert_eq!(s.current_workspace().layout.index(), 1);
    s.reset_layout();
    assert_eq!(s.current_workspace().layout.index(), 0);
}

/// A tiny deterministic xorshift generator for the random-operation test.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

#[test]
fn invariants_hold_under_random_operations() {
    for seed in 1..=40u64 {
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let mut s = new_set(5);
        let mut managed: BTreeSet<WindowId> = BTreeSet::new();
        let mut next_window = 1;
        for _ in 0..400 {
            let tag = (rng.below(7) + 1).to_string();
            let screen = ScreenId(rng.below(4));
            match rng.below(22) {
                0 | 1 => {
                    s.insert_up(w(next_window));
                    managed.insert(w(next_window));
                    next_window += 1;
                }
                2 => {
                    if let Some(&victim) = managed
                        .iter()
                        .nth(usize::try_from(rng.below(managed.len().max(1) as u64)).unwrap_or(0))
                    {
                        s.delete(victim);
                        managed.remove(&victim);
                    }
                }
                3 => s.view(&tag),
                4 => s.greedy_view(&tag),
                5 => s.shift(&tag),
                6 => s.copy(&tag),
                7 => s.focus_down(),
                8 => s.swap_up(),
                9 => s.swap_master(),
                10 => {
                    let x = i32::try_from(rng.below(3)).unwrap_or(0) * 1000;
                    s.add_screen(screen, "out", Rect::new(x, 0, 1000, 800));
                }
                11 => s.remove_screen(screen),
                12 => {
                    s.add_workspace(&tag);
                }
                13 => {
                    s.rename_current(&tag);
                }
                14 => {
                    s.remove_current_workspace();
                }
                15 => s.toggle_ws(&[]),
                16 => s.swap_next_screen(),
                17 => {
                    if let Some(id) = s.neighbour(Direction::Next) {
                        s.focus_screen(id);
                    }
                }
                18 => {
                    if let Some(p) = s.peek() {
                        s.float(p, full_rr());
                    }
                }
                19 => {
                    if let Some(p) = s.peek() {
                        s.sink(p);
                    }
                }
                20 => {
                    if let Some(&target) = managed.iter().next() {
                        s.focus_window(target);
                    }
                }
                _ => s.focus_up(),
            }
            check_invariants(&s);
            // Every managed window stays on at least one workspace.
            let members: BTreeSet<WindowId> = s.all_windows().into_iter().collect();
            assert_eq!(members, managed, "seed {seed}");
        }
    }
}
