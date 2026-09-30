//! Snapshot → restore round trips (restart in place).

use super::*;
use crate::{
    layout::Layout,
    prompt::{
        PromptKey,
        PromptKind,
    },
    stack_set::SCRATCHPAD_TAG,
};

/// Offset of the window ids the "new process" assigns.
const NEW: u64 = 100;

/// Names a window by its original number: ids `n` and `n + NEW` are the
/// same window before and after the restart.
#[expect(
    clippy::unnecessary_wraps,
    reason = "the `id_of` callback of `Wm::snapshot`"
)]
fn ident_of(id: WindowId) -> Option<String> {
    Some(format!("id{}", id.0 % NEW))
}

fn ident(n: u64) -> String {
    format!("id{n}")
}

fn snap(wm: &Wm) -> Snapshot {
    wm.snapshot(ident_of)
}

/// Window number of the scratchpad terminal.
const SCRATCH: u64 = 6;

/// The info the test gives window `n` (restored with the same info).
fn window_info(n: u64) -> WindowInfo {
    if n == SCRATCH {
        info("scratchpad", "term")
    } else {
        info("app", &format!("window {n}"))
    }
}

fn type_text(wm: &mut Wm, text: &str) {
    for c in text.chars() {
        wm.prompt_key(PromptKey::Char(c));
    }
    wm.prompt_key(PromptKey::Return);
}

fn prompt(wm: &mut Wm, kind: PromptKind, text: &str) {
    wm.apply(Action::Prompt(kind));
    type_text(wm, text);
}

/// Restores `snapshot` into a fresh `Wm`: adds `screens` (`(id, name,
/// rect)`), manages windows `order` (original numbers) with new ids and
/// finishes the restore.
fn restore_with(snapshot: Snapshot, screens: &[(u64, &str, Rect)], order: &[u64]) -> Wm {
    let mut wm = Wm::new(Config::defaults());
    wm.restore(snapshot).expect("snapshot restores");
    assert!(wm.is_restoring());
    for &(id, name, rect) in screens {
        wm.add_output(OutputId(id), name, rect);
    }
    for &n in order {
        wm.manage_as(w(n + NEW), Some(&ident(n)), window_info(n));
    }
    wm.finish_restore();
    assert!(!wm.is_restoring());
    wm
}

/// A screen's name and tab strip (tabs as `(window, focused, rect)`).
type NormalizedScreen = (String, Option<(Rect, Vec<(String, bool, Rect)>)>);

/// Everything visible in an arrangement with window and screen ids
/// replaced by stable names, so arrangements of the original and the
/// restored `Wm` compare equal.
#[derive(Debug, PartialEq, Eq)]
struct Normalized {
    windows: Vec<(String, String, Rect, Layer, Option<Border>)>,
    hidden: Vec<String>,
    focus: Option<String>,
    screens: Vec<NormalizedScreen>,
    bar: Option<Rect>,
}

fn normalized(wm: &Wm) -> Normalized {
    let a = wm.arrange();
    let screen_name = |id: ScreenId| {
        wm.stack_set()
            .screen(id)
            .map(|s| s.name.clone())
            .unwrap_or_default()
    };
    let name = |id: WindowId| ident_of(id).unwrap_or_default();
    let mut windows: Vec<_> = a
        .windows
        .iter()
        .map(|p| {
            (
                name(p.window),
                screen_name(p.screen),
                p.rect,
                p.layer,
                p.border,
            )
        })
        .collect();
    // Screen order follows the (new) ids; keep bottom-to-top per screen.
    windows.sort_by(|a, b| a.1.cmp(&b.1));
    let mut hidden: Vec<String> = a.hidden.iter().map(|&w| name(w)).collect();
    hidden.sort();
    let mut screens: Vec<_> = a
        .screens
        .iter()
        .map(|s| {
            (
                screen_name(s.screen),
                s.tab_strip.as_ref().map(|t| {
                    (
                        t.rect,
                        t.tabs
                            .iter()
                            .map(|tab| (name(tab.window), tab.focused, tab.rect))
                            .collect(),
                    )
                }),
            )
        })
        .collect();
    screens.sort_by(|a, b| a.0.cmp(&b.0));
    Normalized {
        windows,
        hidden,
        focus: match a.focus {
            Focus::Window(id) => Some(name(id)),
            Focus::Prompt => Some("prompt".into()),
            Focus::LayoutPicker => Some("layout picker".into()),
            Focus::Help => Some("help".into()),
            Focus::None => None,
        },
        screens,
        bar: a.bar.map(|(_, rect)| rect),
    }
}

/// Asserts both `Wm`s have the same state: the snapshots (workspace
/// order, windows, focus, layouts, floats, fullscreen, bar, screens by
/// name) and the arrangement of every workspace when viewed.
fn assert_same(original: &Wm, restored: &Wm) {
    let mut expected = snap(original);
    let mut actual = snap(restored);
    // Screen order follows the ids, which may differ.
    expected.screens.sort_by(|a, b| a.name.cmp(&b.name));
    actual.screens.sort_by(|a, b| a.name.cmp(&b.name));
    assert_eq!(actual, expected);
    assert_eq!(normalized(restored), normalized(original));
    assert_eq!(
        restored
            .stack_set()
            .current_screen()
            .map(|s| s.name.clone()),
        original
            .stack_set()
            .current_screen()
            .map(|s| s.name.clone())
    );
    for workspace in original.stack_set().workspaces() {
        let tag = &workspace.tag;
        let (mut a, mut b) = (original.clone(), restored.clone());
        a.stack_set.greedy_view(tag);
        b.stack_set.greedy_view(tag);
        assert_eq!(normalized(&b), normalized(&a), "workspace {tag}");
    }
}

/// Windows of workspace `tag` as original numbers' identifiers.
fn idents_on(wm: &Wm, tag: &str) -> Vec<String> {
    wm.stack_set()
        .workspace(tag)
        .map(|w| w.windows().into_iter().filter_map(ident_of).collect())
        .unwrap_or_default()
}

const DP1: (u64, &str, Rect) = (0, "DP-1", SCREEN);
const DP2: (u64, &str, Rect) = (1, "DP-2", Rect::new(1920, 0, 1920, 1080));

/// A single-screen `Wm` in a non-trivial state: layouts changed on two
/// workspaces, a float, a fullscreen window, a copy, a scratchpad hidden
/// on `NSP`, a dynamic and a renamed workspace, the bar hidden and the
/// focus not on the master.
fn busy_single() -> Wm {
    let mut wm = wm();
    for n in 1..=5 {
        wm.manage(w(n), window_info(n));
    }
    // Workspace 1: [5, 4, 3, 2, 1]; resized master, two masters.
    apply(&mut wm, "shrink");
    apply(&mut wm, "shrink");
    apply(&mut wm, "inc-master 1");
    apply(&mut wm, "mirror-expand");
    // Float window 2 somewhere.
    wm.float_at(w(2), Rect::new(300, 200, 640, 480));
    // Window 4 to workspace 2, which uses Mirror tiled.
    wm.focus_window(w(4));
    apply(&mut wm, "shift 2");
    apply(&mut wm, "view 2");
    apply(&mut wm, "next-layout");
    apply(&mut wm, "next-layout");
    apply(&mut wm, "expand");
    wm.request_fullscreen(w(4), true);
    apply(&mut wm, "view 1");
    // Copy window 3 to workspace 3.
    wm.focus_window(w(3));
    apply(&mut wm, "copy 3");
    // A scratchpad, hidden on NSP.
    wm.manage(w(SCRATCH), window_info(SCRATCH));
    apply(&mut wm, "toggle-scratchpad term");
    // A dynamic workspace with a window, renamed, and an empty one.
    prompt(&mut wm, PromptKind::ViewWorkspace, "web");
    wm.manage(w(7), window_info(7));
    prompt(&mut wm, PromptKind::RenameWorkspace, "www");
    prompt(&mut wm, PromptKind::ViewWorkspace, "mail");
    apply(&mut wm, "view 1");
    apply(&mut wm, "toggle-bar");
    // Focus on a non-master window.
    wm.focus_window(w(3));
    wm
}

#[test]
fn busy_single_screen_round_trips() {
    let original = busy_single();
    // Sanity: the state is as intended.
    assert!(original.stack_set().has_tag("www"));
    assert!(!original.stack_set().has_tag("web"));
    assert!(original.stack_set().has_tag("mail"));
    assert!(!original.bar_visible());
    assert_eq!(idents_on(&original, "3"), [ident(3)]);
    assert_eq!(idents_on(&original, SCRATCHPAD_TAG), [ident(6)]);

    let restored = restore_with(snap(&original), &[DP1], &[7, 6, 5, 4, 3, 2, 1]);
    assert_same(&original, &restored);
    assert_eq!(restored.stack_set().current_tag(), "1");
    assert_eq!(restored.stack_set().peek(), Some(w(3 + NEW)));
    assert!(restored.stack_set().is_floating(w(2 + NEW)));
    assert!(restored.is_fullscreen(w(4 + NEW)));
    assert!(!restored.bar_visible());
    assert_eq!(idents_on(&restored, "3"), [ident(3)]);
    assert_eq!(idents_on(&restored, SCRATCHPAD_TAG), [ident(6)]);
    assert!(restored.stack_set().is_floating(w(6 + NEW)));
    assert_eq!(idents_on(&restored, "www"), [ident(7)]);
    let hidden = |wm: &Wm| -> Vec<String> {
        wm.stack_set()
            .hidden()
            .iter()
            .map(|w| w.tag.clone())
            .collect()
    };
    assert_eq!(hidden(&restored), hidden(&original));
    // The scratchpad keeps working.
    let mut restored = restored;
    apply(&mut restored, "toggle-scratchpad term");
    assert!(
        restored
            .stack_set()
            .current_workspace()
            .contains(w(6 + NEW))
    );
}

#[test]
fn restore_is_independent_of_window_order() {
    let original = busy_single();
    let forward = restore_with(snap(&original), &[DP1], &[1, 2, 3, 4, 5, 6, 7]);
    let shuffled = restore_with(snap(&original), &[DP1], &[4, 7, 1, 6, 3, 5, 2]);
    assert_same(&original, &forward);
    assert_same(&original, &shuffled);
}

#[test]
fn restore_survives_a_json_round_trip() {
    let original = busy_single();
    let json = serde_json::to_string(&snap(&original)).expect("serializes");
    let read: Snapshot = serde_json::from_str(&json).expect("deserializes");
    assert_eq!(read, snap(&original));
    let restored = restore_with(read, &[DP1], &[1, 2, 3, 4, 5, 6, 7]);
    assert_same(&original, &restored);
}

#[test]
fn tabbed_and_full_round_trip() {
    let mut original = wm();
    for n in 1..=3 {
        original.manage(w(n), window_info(n));
    }
    original.focus_window(w(2));
    // Tabbed (the fifth layout).
    for _ in 0..4 {
        apply(&mut original, "next-layout");
    }
    assert!(original.arrange().screens[0].tab_strip.is_some());
    let restored = restore_with(snap(&original), &[DP1], &[3, 1, 2]);
    assert_same(&original, &restored);
    assert_eq!(
        restored.stack_set().current_workspace().layout.name(),
        "Tabs"
    );
}

fn busy_dual() -> Wm {
    let mut wm = wm2();
    for n in 1..=3 {
        wm.manage(w(n), window_info(n));
    }
    apply(&mut wm, "next-layout");
    apply(&mut wm, "focus-screen next");
    for n in 4..=5 {
        wm.manage(w(n), window_info(n));
    }
    apply(&mut wm, "expand");
    wm.float_at(w(5), Rect::new(2100, 100, 500, 400));
    wm.focus_window(w(4));
    // Screen DP-2 (current) shows 4 after a greedy swap, DP-1 shows 2.
    apply(&mut wm, "view 4");
    wm.manage(w(6), window_info(6));
    apply(&mut wm, "focus-screen next");
    apply(&mut wm, "view 2");
    apply(&mut wm, "focus-screen next");
    wm
}

fn screen_tags(wm: &Wm) -> Vec<(String, String)> {
    let mut out: Vec<_> = wm
        .stack_set()
        .screens()
        .iter()
        .map(|s| (s.name.clone(), s.workspace.tag.clone()))
        .collect();
    out.sort();
    out
}

/// Two outputs; DP-2 is split in two (4 | 3), its right half current with
/// a fullscreen window, and DP-1 shows 2.
fn busy_split() -> Wm {
    let mut wm = busy_dual();
    apply(&mut wm, "split-screen 2");
    apply(&mut wm, "focus-screen next");
    wm.manage(w(7), window_info(7));
    apply(&mut wm, "toggle-fullscreen");
    wm
}

#[test]
fn split_screens_round_trip() {
    let original = busy_split();
    let s = snap(&original);
    let half = Ratio::new(1, 2);
    let regions: Vec<(&str, RationalRect)> = s
        .screens
        .iter()
        .map(|s| (s.name.as_str(), s.region))
        .collect();
    assert_eq!(
        regions,
        [
            ("DP-1", RationalRect::FULL),
            (
                "DP-2",
                RationalRect::new(Ratio::ZERO, Ratio::ZERO, half, Ratio::ONE)
            ),
            (
                "DP-2",
                RationalRect::new(half, Ratio::ZERO, half, Ratio::ONE)
            ),
        ]
    );
    let json = serde_json::to_string(&s).expect("serializes");
    let read: Snapshot = serde_json::from_str(&json).expect("deserializes");
    let restored = restore_with(read, &[DP2, DP1], &[7, 6, 5, 4, 3, 2, 1]);
    assert_same(&original, &restored);
    assert!(restored.stack_set().is_split(OutputId(DP2.0)));
    let p = restored
        .arrange()
        .placement(w(7 + NEW))
        .cloned()
        .expect("visible");
    assert_eq!(p.layer, Layer::VirtualFullscreen);
    assert_eq!(p.rect, Rect::new(2880, 0, 960, 1080));
}

#[test]
fn a_split_is_not_restored_onto_another_output() {
    let original = busy_split();
    // Only DP-1 comes back: no split, nothing lost.
    let restored = restore_with(snap(&original), &[DP1], &[1, 2, 3, 4, 5, 6, 7]);
    assert_eq!(restored.stack_set().screens().len(), 1);
    assert_eq!(restored.stack_set().all_windows().len(), 7);
}

#[test]
fn snapshots_without_regions_load_unsplit() {
    let original = busy_dual();
    let mut json: serde_json::Value = serde_json::to_value(snap(&original)).expect("serializes");
    for screen in json["screens"].as_array_mut().expect("screens") {
        screen.as_object_mut().expect("object").remove("region");
    }
    let read: Snapshot = serde_json::from_value(json).expect("old format loads");
    assert!(read.screens.iter().all(|s| s.region == RationalRect::FULL));
    let restored = restore_with(read, &[DP1, DP2], &[6, 5, 4, 3, 2, 1]);
    assert_same(&original, &restored);
}

#[test]
fn invalid_regions_are_ignored() {
    let mut s = snap(&busy_split());
    s.screens[2].region.w = Ratio::new(3, 2);
    let restored = restore_with(s, &[DP1, DP2], &[1, 2, 3, 4, 5, 6, 7]);
    assert!(!restored.stack_set().is_split(OutputId(DP2.0)));
    assert_eq!(restored.stack_set().all_windows().len(), 7);
}

#[test]
fn two_screens_round_trip() {
    let original = busy_dual();
    assert_eq!(
        screen_tags(&original),
        [("DP-1".into(), "2".into()), ("DP-2".into(), "4".into())]
    );
    assert_eq!(
        original.stack_set().current_screen().map(|s| s.id),
        Some(S1)
    );
    let restored = restore_with(snap(&original), &[DP1, DP2], &[6, 5, 4, 3, 2, 1]);
    assert_same(&original, &restored);
}

#[test]
fn screens_match_by_name_whatever_their_ids() {
    let original = busy_dual();
    // New ids, DP-2 announced first. (The bar goes to the screen with
    // the lowest id, so the ids keep the physical order.)
    let restored = restore_with(
        snap(&original),
        &[
            (9, "DP-2", Rect::new(1920, 0, 1920, 1080)),
            (7, "DP-1", SCREEN),
        ],
        &[1, 2, 3, 4, 5, 6],
    );
    assert_same(&original, &restored);
    assert_eq!(screen_tags(&restored), screen_tags(&original));
    assert_eq!(
        restored.stack_set().current_screen().map(|s| s.id),
        Some(ScreenId(9))
    );

    // Ids against the physical order: workspaces still follow the names.
    let restored = restore_with(
        snap(&original),
        &[
            (3, "DP-2", Rect::new(1920, 0, 1920, 1080)),
            (4, "DP-1", SCREEN),
        ],
        &[1, 2, 3, 4, 5, 6],
    );
    assert_eq!(screen_tags(&restored), screen_tags(&original));
    assert_eq!(
        restored
            .stack_set()
            .current_screen()
            .map(|s| s.name.as_str()),
        Some("DP-2")
    );
}

#[test]
fn unnamed_or_renamed_screens_match_in_order() {
    let original = busy_dual();
    // Neither name is known: screens pair up in order (DP-1's workspace
    // goes to the first screen).
    let restored = restore_with(
        snap(&original),
        &[
            (0, "HDMI-A-1", SCREEN),
            (1, "", Rect::new(1920, 0, 1920, 1080)),
        ],
        &[1, 2, 3, 4, 5, 6],
    );
    assert_eq!(
        screen_tags(&restored),
        [(String::new(), "4".into()), ("HDMI-A-1".into(), "2".into())]
    );
    assert_eq!(restored.stack_set().current_tag(), "4");

    // One name matches: the other screen gets the remaining workspace.
    let restored = restore_with(
        snap(&original),
        &[
            (0, "HDMI-A-1", SCREEN),
            (1, "DP-1", Rect::new(1920, 0, 1920, 1080)),
        ],
        &[1, 2, 3, 4, 5, 6],
    );
    assert_eq!(
        screen_tags(&restored),
        [("DP-1".into(), "2".into()), ("HDMI-A-1".into(), "4".into())]
    );
    assert_eq!(restored.stack_set().current_tag(), "4");
    assert_eq!(
        restored.stack_set().current_screen().map(|s| s.id),
        Some(ScreenId(0))
    );
}

#[test]
fn fewer_screens_show_the_current_workspace() {
    let original = busy_dual();
    let restored = restore_with(snap(&original), &[DP1], &[1, 2, 3, 4, 5, 6]);
    assert_eq!(screen_tags(&restored), [("DP-1".into(), "4".into())]);
    // Nothing is lost: DP-1's old workspace is hidden with its windows.
    assert_eq!(idents_on(&restored, "2"), idents_on(&original, "2"));
    assert_eq!(idents_on(&restored, "4"), idents_on(&original, "4"));
    assert_eq!(restored.stack_set().all_windows().len(), 6);
}

#[test]
fn more_screens_keep_the_saved_ones() {
    let original = busy_single();
    let restored = restore_with(snap(&original), &[DP1, DP2], &[1, 2, 3, 4, 5, 6, 7]);
    let tags = screen_tags(&restored);
    assert_eq!(tags[0], ("DP-1".into(), "1".into()));
    assert_ne!(tags[1].1, SCRATCHPAD_TAG);
    assert_eq!(restored.stack_set().current_tag(), "1");
}

#[test]
fn headless_restore_keeps_the_current_workspace() {
    let original = busy_single();
    let restored = restore_with(snap(&original), &[], &[1, 2, 3, 4, 5, 6, 7]);
    assert!(restored.stack_set().is_headless());
    assert_eq!(restored.stack_set().current_tag(), "1");
    assert_eq!(idents_on(&restored, "1"), idents_on(&original, "1"));
}

#[test]
fn missing_windows_are_forgotten() {
    let mut original = wm();
    for n in 1..=4 {
        original.manage(w(n), window_info(n));
    }
    original.focus_window(w(2));
    apply(&mut original, "copy 2");
    original.float_at(w(2), Rect::new(10, 10, 100, 100));
    original.request_fullscreen(w(2), true);
    // [4, 3, 2, 1] focused on 2, which never comes back.
    let mut restored = restore_with(snap(&original), &[DP1], &[1, 3, 4]);
    assert_eq!(idents_on(&restored, "1"), [ident(4), ident(3), ident(1)]);
    assert_eq!(idents_on(&restored, "2"), Vec::<String>::new());
    assert!(
        restored
            .stack_set()
            .workspace("2")
            .is_some_and(Workspace::is_empty)
    );
    let focus = restored.stack_set().peek().expect("a focused window");
    assert!(restored.is_managed(focus));
    assert!(restored.stack_set().floating().is_empty());
    assert!(!restored.is_fullscreen(w(2 + NEW)));
    assert_eq!(restored.stack_set().all_windows().len(), 3);

    // A window with that identifier appearing later is a new window.
    restored.manage_as(w(2 + NEW), Some(&ident(2)), window_info(2));
    assert!(!restored.stack_set().is_floating(w(2 + NEW)));
    assert!(!restored.is_fullscreen(w(2 + NEW)));
    assert_eq!(idents_on(&restored, "2"), Vec::<String>::new());
    assert_eq!(restored.stack_set().peek(), Some(w(2 + NEW)));
}

#[test]
fn empty_workspaces_have_no_focus() {
    let mut original = wm();
    original.manage(w(1), window_info(1));
    let restored = restore_with(snap(&original), &[DP1], &[]);
    assert!(restored.stack_set().current_workspace().is_empty());
    assert_eq!(restored.arrange().focus, Focus::None);
}

#[test]
fn unknown_windows_are_managed_with_the_rules() {
    let mut original = wm();
    for n in 1..=2 {
        original.manage(w(n), window_info(n));
    }
    apply(&mut original, "shift 2");
    // [1] on "1", [2] on "2".
    let mut wm = Wm::new(Config::defaults());
    wm.restore(snap(&original)).expect("restores");
    wm.add_output(O0, "DP-1", SCREEN);
    wm.manage_as(w(101), Some(&ident(1)), window_info(1));
    // Unknown identifier, no identifier, a rule-matched window.
    wm.manage_as(w(150), Some("fresh"), window_info(50));
    wm.manage_as(w(151), None, window_info(51));
    wm.manage_as(w(152), Some("zoom"), info("zoom", "Settings"));
    wm.manage_as(w(102), Some(&ident(2)), window_info(2));
    wm.finish_restore();
    // Restored windows first, in snapshot order; new ones keep their
    // relative order below.
    assert_eq!(
        wm.stack_set().workspace("2").map(Workspace::windows),
        Some(vec![w(102)])
    );
    assert_eq!(windows(&wm), [w(101), w(152), w(151), w(150)]);
    // The zoom rule floated its window; restored windows did not float.
    assert!(wm.stack_set().is_floating(w(152)));
    assert!(!wm.stack_set().is_floating(w(101)));
    // The restored focus wins.
    assert_eq!(wm.stack_set().peek(), Some(w(101)));
}

#[test]
fn a_duplicate_identifier_is_managed_normally() {
    let mut original = wm();
    original.manage(w(1), window_info(1));
    apply(&mut original, "shift 2");
    let mut wm = Wm::new(Config::defaults());
    wm.restore(snap(&original)).expect("restores");
    wm.add_output(O0, "DP-1", SCREEN);
    wm.manage_as(w(101), Some(&ident(1)), window_info(1));
    wm.manage_as(w(201), Some(&ident(1)), window_info(1));
    wm.finish_restore();
    assert_eq!(
        wm.stack_set().workspace("2").map(Workspace::windows),
        Some(vec![w(101)])
    );
    assert_eq!(windows(&wm), [w(201)]);
}

#[test]
fn managing_a_known_window_again_updates_its_info() {
    let mut original = wm();
    original.manage(w(1), window_info(1));
    let mut wm = Wm::new(Config::defaults());
    wm.restore(snap(&original)).expect("restores");
    wm.add_output(O0, "DP-1", SCREEN);
    wm.manage_as(w(101), Some(&ident(1)), window_info(1));
    wm.manage_as(w(101), Some(&ident(1)), info("app", "renamed"));
    wm.finish_restore();
    assert_eq!(windows(&wm), [w(101)]);
    assert_eq!(
        wm.window_info(w(101)).map(|i| i.title.as_str()),
        Some("renamed")
    );
}

#[test]
fn manage_as_without_a_restore_is_manage() {
    let mut a = wm();
    let mut b = wm();
    for n in 1..=3 {
        a.manage(w(n), window_info(n));
        b.manage_as(w(n), Some(&ident(n)), window_info(n));
    }
    b.manage_as(w(9), None, info("zoom", "x"));
    a.manage(w(9), info("zoom", "x"));
    assert_eq!(snap(&a), snap(&b));
    // finish_restore without a restore does nothing.
    b.finish_restore();
    assert_eq!(snap(&a), snap(&b));
}

#[test]
fn identifier_less_windows_are_left_out() {
    let mut original = wm();
    for n in 1..=3 {
        original.manage(w(n), window_info(n));
    }
    original.float_at(w(2), Rect::new(0, 0, 50, 50));
    let snapshot = original.snapshot(|id| (id != w(2)).then(|| format!("id{}", id.0)));
    assert_eq!(snapshot.workspaces[0].windows, [ident(3), ident(1)]);
    assert!(snapshot.floating.is_empty());
}

#[test]
fn snapshot_describes_the_state() {
    let original = busy_dual();
    let s = snap(&original);
    assert_eq!(s.version, Snapshot::VERSION);
    assert_eq!(s.current, "4");
    assert_eq!(
        s.screens,
        [
            ScreenSnapshot {
                name: "DP-1".into(),
                workspace: "2".into(),
                region: RationalRect::FULL,
            },
            ScreenSnapshot {
                name: "DP-2".into(),
                workspace: "4".into(),
                region: RationalRect::FULL,
            },
        ]
    );
    // Current first, then the other screen's, then hidden.
    assert_eq!(s.workspaces[0].tag, "4");
    assert_eq!(s.workspaces[1].tag, "2");
    assert_eq!(s.workspaces[0].focus, Some(ident(6)));
    assert!(s.floating.contains_key(&ident(5)));
    assert!(s.bar_overrides.is_empty());
}

#[test]
fn configured_workspaces_missing_from_the_snapshot_are_added() {
    let original = wm();
    let mut s = snap(&original);
    s.workspaces.retain(|w| w.tag != "9");
    let mut wm = Wm::new(Config::defaults());
    wm.restore(s).expect("restores");
    assert!(wm.stack_set().has_tag("9"));
    // Added last.
    assert_eq!(
        wm.stack_set().hidden().last().map(|w| w.tag.as_str()),
        Some("9")
    );
}

#[test]
fn renamed_configured_workspaces_come_back_like_on_reload() {
    let mut original = wm();
    original.manage(w(1), window_info(1));
    prompt(&mut original, PromptKind::RenameWorkspace, "main");
    let restored = restore_with(snap(&original), &[DP1], &[1]);
    assert_eq!(restored.stack_set().current_tag(), "main");
    assert_eq!(idents_on(&restored, "main"), [ident(1)]);
    // Restarting reloads the configuration, which adds "1" again.
    assert!(
        restored
            .stack_set()
            .workspace("1")
            .is_some_and(Workspace::is_empty)
    );
}

#[test]
fn restore_rejects_bad_snapshots() {
    let original = busy_single();
    let restore = |edit: &dyn Fn(&mut Snapshot)| {
        let mut s = snap(&original);
        edit(&mut s);
        Wm::new(Config::defaults()).restore(s)
    };
    assert_eq!(restore(&|s| s.version = 99), Err(RestoreError::Version(99)));
    assert!(matches!(
        restore(&|s| s.current = "nope".into()),
        Err(RestoreError::Invalid(_))
    ));
    assert!(matches!(
        restore(&|s| s.workspaces[1].tag = s.workspaces[0].tag.clone()),
        Err(RestoreError::Invalid(_))
    ));
    assert!(matches!(
        restore(&|s| s.workspaces[2].tag = String::new()),
        Err(RestoreError::Invalid(_))
    ));
    assert_eq!(restore(&|_| {}), Ok(()));
}

#[test]
fn a_rejected_snapshot_leaves_the_wm_untouched() {
    let mut wm = wm();
    wm.manage(w(1), window_info(1));
    let before = snap(&wm);
    let mut s = before.clone();
    s.version = 0;
    assert!(wm.restore(s).is_err());
    assert!(!wm.is_restoring());
    assert_eq!(snap(&wm), before);
}

#[test]
fn incompatible_layout_rings_fall_back_to_the_configuration() {
    let mut original = wm();
    original.manage(w(1), window_info(1));
    let mut s = snap(&original);
    let mut old = LayoutRing::new(vec![Layout::Full, Layout::Grid]);
    old.next();
    s.workspaces[0].layout = Some(old);
    let restored = restore_with(s, &[DP1], &[1]);
    let ring = &restored.stack_set().current_workspace().layout;
    let template = LayoutRing::standard(Config::defaults().tabs.height);
    assert_eq!(ring.len(), template.len());
    assert_eq!(ring.name(), "Grid");
    assert_eq!(ring.index(), 6);
}

#[test]
fn unreadable_layouts_only_reset_the_layout() {
    let mut original = wm();
    original.manage(w(1), window_info(1));
    original.manage(w(2), window_info(2));
    let mut json = serde_json::to_value(snap(&original)).expect("serialize");
    // A layout variant this version doesn't know (e.g. an old name).
    json["workspaces"][0]["layout"] =
        serde_json::json!({ "layouts": [{ "ReflectX": "Grid" }], "current": 0 });
    let s: Snapshot = serde_json::from_value(json).expect("the snapshot still loads");
    assert_eq!(s.workspaces[0].layout, None);
    let restored = restore_with(s, &[DP1], &[1, 2]);
    let ring = &restored.stack_set().current_workspace().layout;
    assert_eq!(*ring, LayoutRing::standard(Config::defaults().tabs.height));
    assert_eq!(
        restored
            .stack_set()
            .current_workspace()
            .stack
            .as_ref()
            .map(crate::stack_set::Stack::len),
        Some(2)
    );
}
