//! Virtual screens: `split-screen` (`layoutSplitScreen`) and `rescreen`.

use super::*;
use crate::{
    config::{
        BarPosition,
        Color,
        SmartBorders,
    },
    prompt::PromptKind,
    stack_set::Screen,
    wm::WorkspaceState,
};

const LEFT: Rect = Rect::new(0, 0, 960, 1080);
const RIGHT: Rect = Rect::new(960, 0, 960, 1080);
/// The tiling areas of `split-screen 2`'s halves below the default bar,
/// either side of the default 3px divider (the left half gives up 1px, the
/// right one 2px).
const LEFT_AREA: Rect = Rect::new(0, 24, 959, 1056);
const RIGHT_AREA: Rect = Rect::new(962, 24, 958, 1056);
/// The default divider of `split-screen 2` below the default bar.
const DIVIDER: Rect = Rect::new(959, 24, 3, 1056);
const NO_DIVIDERS: [Divider; 0] = [];

/// `(workspace, rect)` of every screen in screen order.
fn layout(wm: &Wm) -> Vec<(String, Rect)> {
    wm.stack_set()
        .screens()
        .iter()
        .map(|s| (s.workspace.tag.clone(), s.rect))
        .collect()
}

fn hidden(wm: &Wm) -> Vec<String> {
    wm.stack_set()
        .hidden()
        .iter()
        .map(|w| w.tag.clone())
        .collect()
}

fn current(wm: &Wm) -> &Screen {
    wm.stack_set().current_screen().expect("a current screen")
}

fn sorted_windows(wm: &Wm) -> Vec<WindowId> {
    let mut all = wm.stack_set().all_windows();
    all.sort();
    all
}

#[test]
fn split_keeps_the_current_workspace_left_and_shows_the_most_recently_hidden_right() {
    let mut wm = wm();
    // History: 3 is the most recently hidden workspace.
    apply(&mut wm, "view 3");
    apply(&mut wm, "view 1");
    assert_eq!(hidden(&wm)[..2], ["3", "2"]);
    apply(&mut wm, "split-screen 2");
    assert_eq!(
        layout(&wm),
        [("1".to_owned(), LEFT), ("3".to_owned(), RIGHT)]
    );
    // The current screen stays current and keeps its id.
    assert_eq!(current(&wm).id, S0);
    assert_eq!(hidden(&wm)[..2], ["2", "4"]);
    assert!(wm.stack_set().is_split(O0));
    // Both belong to the same output.
    assert!(wm.stack_set().screens().iter().all(|s| s.output == O0));
}

#[test]
fn split_screens_tile_independently_under_one_bar() {
    let mut wm = wm();
    manage_all(&mut wm, [1, 2]);
    apply(&mut wm, "split-screen 2");
    apply(&mut wm, "focus-screen next");
    assert_eq!(current(&wm).workspace.tag, "2");
    manage_all(&mut wm, [3]);
    let a = wm.arrange();
    // The bar spans the whole output.
    assert_eq!(bars(&a), [(O0, Rect::new(0, 0, 1920, 24))]);
    let right = current(&wm).id;
    assert_eq!(wm.tiling_area(S0), Some(LEFT_AREA));
    assert_eq!(wm.tiling_area(right), Some(RIGHT_AREA));
    assert_eq!(dividers(&a), [(O0, DIVIDER)]);
    // Window 3 alone fills the right half; 1 and 2 share the left. None
    // reaches under the divider.
    assert_eq!(rect_of(&a, 3), Some(RIGHT_AREA.inset(1)));
    let one = a.placement(w(1)).expect("1 visible");
    let two = a.placement(w(2)).expect("2 visible");
    assert_eq!((one.screen, two.screen), (S0, S0));
    assert!(one.rect.x + one.rect.w < 959 && two.rect.x + two.rect.w < 959);
    assert_eq!(a.focus, Focus::Window(w(3)));
    assert_eq!(a.screens.len(), 2);
    assert!(a.screens.iter().all(|s| s.output == O0));
}

#[test]
fn splitting_a_split_screen_divides_its_half() {
    let mut wm = wm();
    apply(&mut wm, "split-screen 2");
    apply(&mut wm, "focus-screen next");
    apply(&mut wm, "split-screen 2");
    assert_eq!(
        layout(&wm),
        [
            ("1".to_owned(), LEFT),
            ("2".to_owned(), Rect::new(960, 0, 480, 1080)),
            ("3".to_owned(), Rect::new(1440, 0, 480, 1080)),
        ]
    );
    assert_eq!(current(&wm).workspace.tag, "2");
    // Physical order walks left to right.
    apply(&mut wm, "focus-screen next");
    assert_eq!(current(&wm).workspace.tag, "3");
    apply(&mut wm, "focus-screen next");
    assert_eq!(current(&wm).workspace.tag, "1");
}

#[test]
fn split_into_more_than_two_gives_equal_columns() {
    let mut wm = wm();
    apply(&mut wm, "split-screen 3");
    assert_eq!(
        layout(&wm),
        [
            ("1".to_owned(), Rect::new(0, 0, 640, 1080)),
            ("2".to_owned(), Rect::new(640, 0, 640, 1080)),
            ("3".to_owned(), Rect::new(1280, 0, 640, 1080)),
        ]
    );
}

#[test]
fn split_rounds_like_two_pane_and_leaves_no_gap() {
    let mut wm = Wm::new(Config::defaults());
    wm.add_output(O0, "DP-1", Rect::new(10, 0, 1921, 1080));
    apply(&mut wm, "split-screen 2");
    // splitHorizontallyBy ½: the left part gets floor(1921 / 2).
    assert_eq!(
        layout(&wm),
        [
            ("1".to_owned(), Rect::new(10, 0, 960, 1080)),
            ("2".to_owned(), Rect::new(970, 0, 961, 1080)),
        ]
    );
}

#[test]
fn split_creates_workspaces_when_none_are_hidden() {
    let mut config = Config::defaults();
    config.workspaces = vec!["web".into()];
    let mut wm = Wm::new(config);
    wm.add_output(O0, "DP-1", SCREEN);
    apply(&mut wm, "split-screen 2");
    assert_eq!(
        layout(&wm),
        [("web".to_owned(), LEFT), ("1".to_owned(), RIGHT)]
    );
}

#[test]
fn split_never_shows_the_scratchpad_workspace() {
    let mut wm = wm();
    wm.manage(w(9), info("scratchpad", "term"));
    apply(&mut wm, "toggle-scratchpad term");
    assert_eq!(hidden(&wm)[0], "NSP");
    apply(&mut wm, "split-screen 2");
    assert_eq!(layout(&wm)[1].0, "2");
}

#[test]
fn split_while_headless_does_nothing() {
    let mut wm = Wm::new(Config::defaults());
    apply(&mut wm, "split-screen 2");
    assert_eq!(wm.stack_set().screens(), []);
    wm.add_output(O0, "DP-1", SCREEN);
    assert_eq!(layout(&wm), [("1".to_owned(), SCREEN)]);
}

#[test]
fn rescreen_merges_back_keeping_the_current_workspace() {
    let mut wm = wm();
    manage_all(&mut wm, [1]);
    apply(&mut wm, "split-screen 2");
    apply(&mut wm, "focus-screen next");
    manage_all(&mut wm, [2]);
    let before = sorted_windows(&wm);
    apply(&mut wm, "rescreen");
    // The right screen was current: its workspace keeps the output, the
    // left one's becomes the most recently hidden.
    assert_eq!(layout(&wm), [("2".to_owned(), SCREEN)]);
    assert_eq!(current(&wm).id, S0);
    assert_eq!(hidden(&wm)[..3], ["1", "3", "4"]);
    assert_eq!(sorted_windows(&wm), before);
    assert!(!wm.stack_set().is_split(O0));
    // toggle-ws brings the other half's workspace back.
    apply(&mut wm, "toggle-ws");
    assert_eq!(current(&wm).workspace.tag, "1");
    assert_eq!(wm.arrange().placement(w(1)).map(|p| p.screen), Some(S0));
}

#[test]
fn rescreen_puts_removed_workspaces_in_screen_order() {
    let mut wm = wm();
    apply(&mut wm, "split-screen 3");
    apply(&mut wm, "focus-screen next");
    apply(&mut wm, "rescreen");
    assert_eq!(layout(&wm), [("2".to_owned(), SCREEN)]);
    assert_eq!(hidden(&wm)[..3], ["1", "3", "4"]);
}

#[test]
fn rescreen_resets_every_output() {
    let mut wm = wm2();
    // Output 0: 1 | 3; output 1: 2 | 4, with the current screen on the
    // left half of output 1.
    apply(&mut wm, "split-screen 2");
    apply(&mut wm, "focus-screen prev");
    assert_eq!(current(&wm).output, O1);
    apply(&mut wm, "split-screen 2");
    assert_eq!(
        layout(&wm),
        [
            ("1".to_owned(), LEFT),
            ("3".to_owned(), RIGHT),
            ("2".to_owned(), Rect::new(1920, 0, 960, 1080)),
            ("4".to_owned(), Rect::new(2880, 0, 960, 1080)),
        ]
    );
    apply(&mut wm, "rescreen");
    assert_eq!(
        layout(&wm),
        [
            ("1".to_owned(), SCREEN),
            ("2".to_owned(), Rect::new(1920, 0, 1920, 1080)),
        ]
    );
    let ids: Vec<ScreenId> = wm.stack_set().screens().iter().map(|s| s.id).collect();
    assert_eq!(ids, [S0, S1]);
    assert_eq!(current(&wm).id, S1);
    assert_eq!(hidden(&wm)[..3], ["3", "4", "5"]);
}

#[test]
fn rescreen_without_splits_changes_nothing() {
    let mut wm = wm2();
    apply(&mut wm, "view 5");
    let before = wm.stack_set().clone();
    apply(&mut wm, "rescreen");
    assert_eq!(wm.stack_set(), &before);
}

#[test]
fn usable_area_and_bar_apply_to_each_virtual_screen() {
    let mut wm = wm();
    apply(&mut wm, "split-screen 2");
    wm.set_usable_rect(O0, Rect::new(0, 30, 1920, 1050));
    let right = wm.stack_set().screens()[1].id;
    assert_eq!(
        wm.stack_set().screens()[0].usable_rect,
        Rect::new(0, 30, 960, 1050)
    );
    assert_eq!(bars(&wm.arrange()), [(O0, Rect::new(0, 30, 1920, 24))]);
    assert_eq!(wm.tiling_area(S0), Some(Rect::new(0, 54, 959, 1026)));
    assert_eq!(wm.tiling_area(right), Some(Rect::new(962, 54, 958, 1026)));
    // The divider starts below the bar, inside the usable area.
    assert_eq!(dividers(&wm.arrange()), [(O0, Rect::new(959, 54, 3, 1026))]);
}

#[test]
fn bar_on_another_output_leaves_the_split_alone() {
    let mut config = Config::defaults();
    config.bar.output = Some(vec!["DP-1".into()]);
    let mut wm = Wm::new(config);
    wm.add_output(O0, "DP-1", SCREEN);
    wm.add_output(O1, "DP-2", Rect::new(1920, 0, 1920, 1080));
    apply(&mut wm, "focus-screen next");
    apply(&mut wm, "split-screen 2");
    let a = wm.arrange();
    assert_eq!(bars(&a), [(O0, Rect::new(0, 0, 1920, 24))]);
    // The halves of O1 only give up the divider, which spans the whole
    // height.
    let halves: Vec<Option<Rect>> = wm
        .stack_set()
        .screens()
        .iter()
        .filter(|s| s.output == O1)
        .map(|s| wm.tiling_area(s.id))
        .collect();
    assert_eq!(
        halves,
        [
            Some(Rect::new(1920, 0, 959, 1080)),
            Some(Rect::new(2882, 0, 958, 1080)),
        ]
    );
    assert_eq!(dividers(&a), [(O1, Rect::new(2879, 0, 3, 1080))]);
}

#[test]
fn fullscreen_on_a_virtual_screen_covers_only_that_screen() {
    let mut wm = wm();
    apply(&mut wm, "split-screen 2");
    apply(&mut wm, "focus-screen next");
    manage_all(&mut wm, [1]);
    apply(&mut wm, "toggle-fullscreen");
    let a = wm.arrange();
    let p = a.placement(w(1)).expect("visible");
    assert_eq!(p.layer, Layer::VirtualFullscreen);
    assert_eq!(p.rect, RIGHT);
    assert_eq!(p.output, O0);
    assert_eq!(p.border, None);
    // Unsplit again, it is fullscreen on the output.
    apply(&mut wm, "rescreen");
    let a = wm.arrange();
    let p = a.placement(w(1)).expect("visible");
    assert_eq!(p.layer, Layer::Fullscreen);
    assert_eq!(p.rect, SCREEN);
}

#[test]
fn a_split_output_keeps_its_proportions_when_resized() {
    let mut wm = wm();
    apply(&mut wm, "split-screen 2");
    apply(&mut wm, "focus-screen next");
    wm.set_output_rect(O0, Rect::new(0, 0, 2560, 1440));
    assert_eq!(
        layout(&wm),
        [
            ("1".to_owned(), Rect::new(0, 0, 1280, 1440)),
            ("2".to_owned(), Rect::new(1280, 0, 1280, 1440)),
        ]
    );
    // Announcing the output again (river's dimensions event) also keeps
    // the split, and the current screen.
    wm.add_output(O0, "DP-1", SCREEN);
    assert_eq!(
        layout(&wm),
        [("1".to_owned(), LEFT), ("2".to_owned(), RIGHT)]
    );
    assert_eq!(current(&wm).workspace.tag, "2");
}

#[test]
fn removing_a_split_output_hides_its_workspaces() {
    let mut wm = wm2();
    manage_all(&mut wm, [1]);
    apply(&mut wm, "focus-screen next");
    apply(&mut wm, "split-screen 2");
    manage_all(&mut wm, [2]);
    apply(&mut wm, "focus-screen next");
    manage_all(&mut wm, [3]);
    assert_eq!(current(&wm).workspace.tag, "3");
    let before = sorted_windows(&wm);
    wm.remove_output(O1);
    assert_eq!(layout(&wm), [("1".to_owned(), SCREEN)]);
    assert_eq!(current(&wm).id, S0);
    assert_eq!(hidden(&wm)[..2], ["2", "3"]);
    assert_eq!(sorted_windows(&wm), before);
    // A new output gets a single screen.
    wm.add_output(O1, "DP-2", Rect::new(1920, 0, 1920, 1080));
    assert!(!wm.stack_set().is_split(O1));
    assert_eq!(layout(&wm)[1].0, "2");
}

#[test]
fn removing_the_last_split_output_keeps_the_current_workspace() {
    let mut wm = wm();
    apply(&mut wm, "split-screen 2");
    apply(&mut wm, "focus-screen next");
    wm.remove_output(O0);
    assert!(wm.stack_set().is_headless());
    assert_eq!(wm.stack_set().current_tag(), "2");
    assert_eq!(hidden(&wm)[0], "1");
    wm.add_output(O0, "DP-1", SCREEN);
    assert_eq!(layout(&wm), [("2".to_owned(), SCREEN)]);
}

#[test]
fn pointer_and_floats_pick_the_virtual_screen_by_point() {
    let mut wm = wm();
    manage_all(&mut wm, [1]);
    apply(&mut wm, "split-screen 2");
    let right = wm.stack_set().screens()[1].id;
    wm.focus_screen_at(1500, 500);
    assert_eq!(current(&wm).id, right);
    wm.focus_screen_at(100, 500);
    assert_eq!(current(&wm).id, S0);
    // Dragging window 1 into the right half moves it to workspace 2.
    wm.float_at(w(1), Rect::new(1200, 100, 400, 300));
    assert_eq!(wm.stack_set().find_tag(w(1)), Some("2"));
    assert_eq!(current(&wm).id, right);
    let p = wm.arrange().placement(w(1)).cloned().expect("visible");
    assert_eq!((p.screen, p.rect), (right, Rect::new(1200, 100, 400, 300)));
}

#[test]
fn screen_actions_work_across_virtual_screens() {
    let mut wm = wm();
    manage_all(&mut wm, [1]);
    apply(&mut wm, "split-screen 2");
    // shift-screen sends the window to the right half's workspace.
    apply(&mut wm, "shift-screen next");
    assert_eq!(wm.stack_set().find_tag(w(1)), Some("2"));
    assert_eq!(current(&wm).id, S0);
    // swap-next-screen swaps the halves' workspaces.
    apply(&mut wm, "swap-next-screen");
    assert_eq!(
        layout(&wm),
        [("2".to_owned(), LEFT), ("1".to_owned(), RIGHT)]
    );
    // greedy view of the other half's workspace swaps too.
    apply(&mut wm, "view 1");
    assert_eq!(
        layout(&wm),
        [("1".to_owned(), LEFT), ("2".to_owned(), RIGHT)]
    );
    // The bar shows the other half's workspace as visible.
    let bar = wm.bar_model(O0);
    assert_eq!(bar.workspaces[0], ("1".to_owned(), WorkspaceState::Current));
    assert_eq!(bar.workspaces[1], ("2".to_owned(), WorkspaceState::Visible));
}

#[test]
fn scratchpad_and_prompt_use_the_current_virtual_screen() {
    let mut wm = wm();
    apply(&mut wm, "split-screen 2");
    apply(&mut wm, "focus-screen next");
    wm.manage(w(9), info("scratchpad", "term"));
    let pad = wm.arrange().placement(w(9)).cloned().expect("visible");
    // rect = [0.1, 0.1, 0.8, 0.8] of the right half.
    assert_eq!(pad.screen, current(&wm).id);
    assert_eq!(pad.rect, Rect::new(1056, 108, 768, 864).inset(1));
    wm.apply(Action::Prompt(PromptKind::ViewWorkspace));
    let (screen, rect) = wm.arrange().prompt.expect("prompt open");
    assert_eq!(screen, current(&wm).id);
    assert_eq!((rect.x, rect.w), (960, 960));
}

#[test]
fn a_split_outputs_bar_speaks_for_its_current_or_first_screen() {
    let mut wm = wm2();
    manage_all(&mut wm, [1]);
    apply(&mut wm, "split-screen 2");
    let left = current(&wm).id;
    apply(&mut wm, "focus-screen next");
    let right = current(&wm).id;
    assert_ne!(left, right);
    assert_eq!(current(&wm).output, O0);
    wm.manage(w(2), info("app", "right half"));
    // The current screen is the right half: O0's bar shows its window.
    assert_eq!(wm.bar_screen(O0).map(|s| s.id), Some(right));
    assert_eq!(wm.bar_model(O0).title, "right half");
    // Hiding the bar on the right half's workspace hides O0's bar, and
    // both halves tile without it.
    apply(&mut wm, "toggle-bar");
    assert!(!wm.bar_shown_on(O0));
    assert_eq!(wm.tiling_area(left), Some(Rect::new(0, 0, 959, 1080)));
    // With focus on the other output, the left half speaks for O0 again.
    apply(&mut wm, "focus-screen next");
    assert_eq!(current(&wm).output, O1);
    assert_eq!(wm.bar_screen(O0).map(|s| s.id), Some(left));
    assert_eq!(wm.bar_model(O0).title, "window 1");
    assert!(wm.bar_shown_on(O0));
    assert_eq!(wm.tiling_area(right), Some(RIGHT_AREA));
}

#[test]
fn a_divider_separates_the_halves_of_a_split() {
    let mut wm = wm();
    assert_eq!(wm.arrange().dividers, NO_DIVIDERS);
    apply(&mut wm, "split-screen 2");
    let a = wm.arrange();
    assert_eq!(
        a.dividers,
        [Divider {
            output: O0,
            rect: DIVIDER,
            color: Config::defaults().split.divider_color,
        }]
    );
    // The bar still spans the whole output, above the divider.
    assert_eq!(bars(&a), [(O0, Rect::new(0, 0, 1920, 24))]);
    let right = wm.stack_set().screens()[1].id;
    assert_eq!(wm.tiling_area(S0), Some(LEFT_AREA));
    assert_eq!(wm.tiling_area(right), Some(RIGHT_AREA));
    // The screens themselves keep their halves.
    assert_eq!(
        layout(&wm),
        [("1".to_owned(), LEFT), ("2".to_owned(), RIGHT)]
    );
}

#[test]
fn three_columns_get_two_dividers() {
    let mut wm = wm();
    apply(&mut wm, "split-screen 3");
    assert_eq!(
        dividers(&wm.arrange()),
        [
            (O0, Rect::new(639, 24, 3, 1056)),
            (O0, Rect::new(1279, 24, 3, 1056)),
        ]
    );
    assert_eq!(
        tiling_areas(&wm),
        [
            Some(Rect::new(0, 24, 639, 1056)),
            Some(Rect::new(642, 24, 637, 1056)),
            Some(Rect::new(1282, 24, 638, 1056)),
        ]
    );
}

/// The tiling area of every screen in screen order.
fn tiling_areas(wm: &Wm) -> Vec<Option<Rect>> {
    wm.stack_set()
        .screens()
        .iter()
        .map(|s| wm.tiling_area(s.id))
        .collect()
}

#[test]
fn odd_output_and_divider_widths() {
    let mut config = Config::defaults();
    config.split.divider_width = 4;
    let mut wm = Wm::new(config);
    wm.add_output(O0, "DP-1", Rect::new(10, 0, 1921, 1080));
    apply(&mut wm, "split-screen 2");
    // Halves 960 | 961 wide meeting at x = 970; an even width splits
    // evenly.
    assert_eq!(dividers(&wm.arrange()), [(O0, Rect::new(968, 24, 4, 1056))]);
    assert_eq!(
        tiling_areas(&wm),
        [
            Some(Rect::new(10, 24, 958, 1056)),
            Some(Rect::new(972, 24, 959, 1056)),
        ]
    );
    // An odd width gives the right side the extra pixel.
    let mut config = Config::defaults();
    config.split.divider_width = 5;
    wm.reconfigure(config);
    assert_eq!(dividers(&wm.arrange()), [(O0, Rect::new(968, 24, 5, 1056))]);
    assert_eq!(
        tiling_areas(&wm),
        [
            Some(Rect::new(10, 24, 958, 1056)),
            Some(Rect::new(973, 24, 958, 1056)),
        ]
    );
}

#[test]
fn a_zero_divider_width_keeps_the_halves_touching() {
    let mut config = Config::defaults();
    config.split.divider_width = 0;
    let mut wm = Wm::new(config);
    wm.add_output(O0, "DP-1", SCREEN);
    manage_all(&mut wm, [1]);
    apply(&mut wm, "split-screen 2");
    let a = wm.arrange();
    assert_eq!(a.dividers, NO_DIVIDERS);
    assert_eq!(
        tiling_areas(&wm),
        [
            Some(Rect::new(0, 24, 960, 1056)),
            Some(Rect::new(960, 24, 960, 1056)),
        ]
    );
    assert_eq!(rect_of(&a, 1), Some(Rect::new(0, 24, 960, 1056).inset(1)));
}

#[test]
fn reload_applies_the_divider_settings() {
    let mut wm = wm();
    apply(&mut wm, "split-screen 2");
    let mut config = Config::defaults();
    config.split.divider_width = 0;
    wm.reconfigure(config.clone());
    assert_eq!(wm.arrange().dividers, NO_DIVIDERS);
    assert_eq!(wm.tiling_area(S0), Some(Rect::new(0, 24, 960, 1056)));
    config.split.divider_width = 2;
    config.split.divider_color = Color::rgb(1, 2, 3);
    wm.reconfigure(config);
    let a = wm.arrange();
    assert_eq!(dividers(&a), [(O0, Rect::new(959, 24, 2, 1056))]);
    assert_eq!(a.dividers[0].color, Color::rgb(1, 2, 3));
}

#[test]
fn outputs_never_get_a_divider_between_them() {
    let mut wm = wm2();
    assert_eq!(wm.arrange().dividers, NO_DIVIDERS);
    // Splitting both outputs gives each its own divider, none on x = 1920
    // where the outputs meet.
    apply(&mut wm, "split-screen 2");
    apply(&mut wm, "focus-screen prev");
    assert_eq!(current(&wm).output, O1);
    apply(&mut wm, "split-screen 2");
    assert_eq!(
        dividers(&wm.arrange()),
        [(O0, DIVIDER), (O1, Rect::new(2879, 24, 3, 1056))]
    );
    // The outer edges keep their space.
    assert_eq!(
        tiling_areas(&wm),
        [
            Some(LEFT_AREA),
            Some(RIGHT_AREA),
            Some(Rect::new(1920, 24, 959, 1056)),
            Some(Rect::new(2882, 24, 958, 1056)),
        ]
    );
}

#[test]
fn rescreen_removes_the_divider() {
    let mut wm = wm();
    manage_all(&mut wm, [1]);
    apply(&mut wm, "split-screen 2");
    assert_eq!(wm.arrange().dividers.len(), 1);
    apply(&mut wm, "rescreen");
    let a = wm.arrange();
    assert_eq!(a.dividers, NO_DIVIDERS);
    assert_eq!(wm.tiling_area(S0), Some(AREA));
    assert_eq!(rect_of(&a, 1), Some(AREA));
}

#[test]
fn per_screen_smart_borders_keep_the_divider() {
    let mut config = Config::defaults();
    config.border.smart = SmartBorders::PerScreen;
    let mut wm = Wm::new(config);
    wm.add_output(O0, "DP-1", SCREEN);
    manage_all(&mut wm, [1]);
    apply(&mut wm, "split-screen 2");
    apply(&mut wm, "focus-screen next");
    manage_all(&mut wm, [2]);
    let a = wm.arrange();
    // Each lone window is borderless and fills its half up to the divider.
    for (id, area) in [(1, LEFT_AREA), (2, RIGHT_AREA)] {
        let p = a.placement(w(id)).expect("visible");
        assert_eq!((p.rect, p.border), (area, None));
    }
    assert_eq!(dividers(&a), [(O0, DIVIDER)]);
}

#[test]
fn a_bottom_bar_ends_the_divider_above_it() {
    let mut config = Config::defaults();
    config.bar.position = BarPosition::Bottom;
    let mut wm = Wm::new(config);
    wm.add_output(O0, "DP-1", SCREEN);
    apply(&mut wm, "split-screen 2");
    assert_eq!(dividers(&wm.arrange()), [(O0, Rect::new(959, 0, 3, 1056))]);
    // Without a bar it spans the whole height.
    apply(&mut wm, "toggle-bar");
    assert_eq!(dividers(&wm.arrange()), [(O0, Rect::new(959, 0, 3, 1080))]);
}

#[test]
fn floats_and_fullscreen_windows_ignore_the_divider() {
    let mut wm = wm();
    apply(&mut wm, "split-screen 2");
    wm.manage(w(9), info("scratchpad", "term"));
    // A float is placed in the whole screen: rect = [0.1, 0.1, 0.8, 0.8]
    // of the left half.
    let pad = wm.arrange().placement(w(9)).cloned().expect("visible");
    assert_eq!(pad.rect, Rect::new(96, 108, 768, 864).inset(1));
    // It may cover the divider.
    wm.float_at(w(9), Rect::new(800, 100, 400, 300));
    let pad = wm.arrange().placement(w(9)).cloned().expect("visible");
    assert_eq!(pad.rect, Rect::new(800, 100, 400, 300));
    // A virtual-screen fullscreen window covers the whole half (the
    // right one: the float's move made it current).
    manage_all(&mut wm, [1]);
    apply(&mut wm, "toggle-fullscreen");
    let p = wm.arrange().placement(w(1)).cloned().expect("visible");
    assert_eq!((p.layer, p.rect), (Layer::VirtualFullscreen, RIGHT));
}
