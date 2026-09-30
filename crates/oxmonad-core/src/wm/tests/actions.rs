use super::*;

#[test]
fn spawn_actions_produce_effects() {
    let mut wm = wm();
    assert_eq!(
        apply(&mut wm, "spawn emacs"),
        [Effect::Spawn("emacs".into())]
    );
    assert_eq!(
        apply(&mut wm, "spawn-terminal"),
        [Effect::Spawn("alacritty".into())]
    );
    assert_eq!(apply(&mut wm, "quit"), [Effect::Quit]);
    assert_eq!(apply(&mut wm, "reload"), [Effect::Reload]);
    assert_eq!(apply(&mut wm, "restart"), [Effect::Restart]);
    assert_eq!(apply(&mut wm, "refresh").len(), 0);
}

#[test]
fn close_targets_focused_window() {
    let mut wm = wm();
    assert_eq!(apply(&mut wm, "close").len(), 0);
    manage_all(&mut wm, [1, 2]);
    assert_eq!(apply(&mut wm, "close"), [Effect::Close(w(2))]);
}

#[test]
fn focus_and_swap_actions() {
    let mut wm = wm();
    manage_all(&mut wm, [1, 2, 3]);
    // New windows go above the focus: 3 2 1.
    assert_eq!(windows(&wm), [w(3), w(2), w(1)]);
    apply(&mut wm, "focus-next");
    assert_eq!(wm.stack_set().peek(), Some(w(2)));
    apply(&mut wm, "focus-prev");
    apply(&mut wm, "focus-prev");
    assert_eq!(wm.stack_set().peek(), Some(w(1)));
    apply(&mut wm, "swap-master");
    assert_eq!(windows(&wm), [w(1), w(2), w(3)]);
    apply(&mut wm, "swap-next");
    assert_eq!(windows(&wm), [w(2), w(1), w(3)]);
    apply(&mut wm, "swap-prev");
    assert_eq!(windows(&wm), [w(1), w(2), w(3)]);
    apply(&mut wm, "focus-next");
    apply(&mut wm, "focus-master");
    assert_eq!(wm.stack_set().peek(), Some(w(1)));
}

#[test]
fn sink_tiles_a_floating_window() {
    let mut wm = wm();
    manage_all(&mut wm, [1]);
    wm.float_at(w(1), Rect::new(100, 100, 300, 200));
    assert!(wm.stack_set().is_floating(w(1)));
    apply(&mut wm, "sink");
    assert!(!wm.stack_set().is_floating(w(1)));
}

#[test]
fn layout_cycle_and_reset() {
    let mut wm = wm();
    assert_eq!(wm.bar_model().layout, "ReflectX ResizableTall");
    let expected = [
        "ResizableTall",
        "Mirror ResizableTall",
        "Full",
        "Tabbed Simplest",
        "Mirror Grid",
        "Grid",
        "ThreeCol",
        "ReflectX ThreeCol",
        "ReflectX ResizableTall",
    ];
    for description in expected {
        apply(&mut wm, "next-layout");
        assert_eq!(wm.bar_model().layout, description);
    }
    apply(&mut wm, "next-layout");
    apply(&mut wm, "reset-layout");
    assert_eq!(wm.bar_model().layout, "ReflectX ResizableTall");
}

#[test]
fn layouts_are_per_workspace() {
    let mut wm = wm();
    apply(&mut wm, "next-layout");
    apply(&mut wm, "view 2");
    assert_eq!(wm.bar_model().layout, "ReflectX ResizableTall");
    apply(&mut wm, "view 1");
    assert_eq!(wm.bar_model().layout, "ResizableTall");
}

#[test]
fn shrink_and_expand_change_master_width() {
    let mut wm = wm();
    manage_all(&mut wm, [1, 2]);
    apply(&mut wm, "next-layout"); // plain ResizableTall
    let before = rect_of(&wm.arrange(), 2).expect("placed");
    assert_eq!(before, Rect::new(1, 25, 1150, 1054));
    apply(&mut wm, "expand");
    // frac 3/5 + 3/100 = 63/100 of 1920 = 1209.6 -> 1209.
    assert_eq!(
        rect_of(&wm.arrange(), 2),
        Some(Rect::new(1, 25, 1207, 1054))
    );
    apply(&mut wm, "shrink");
    apply(&mut wm, "shrink");
    // 57/100 of 1920 = 1094.4 -> 1094.
    assert_eq!(
        rect_of(&wm.arrange(), 2),
        Some(Rect::new(1, 25, 1092, 1054))
    );
}

#[test]
fn mirror_resize_changes_slot_heights() {
    let mut wm = wm();
    manage_all(&mut wm, [1, 2, 3]);
    apply(&mut wm, "next-layout");
    // Stack: 3 (master) 2 1; focus 2, the first stack window.
    apply(&mut wm, "focus-next");
    apply(&mut wm, "mirror-shrink");
    let a = wm.arrange();
    // Slot fraction 103/100 of the even 528px split: floor(543.84) = 543.
    assert_eq!(rect_of(&a, 2), Some(Rect::new(1153, 25, 766, 541)));
    assert_eq!(rect_of(&a, 1), Some(Rect::new(1153, 568, 766, 511)));
    apply(&mut wm, "mirror-expand");
    apply(&mut wm, "mirror-expand");
    let a = wm.arrange();
    // 97/100 * 528 = 512.16 -> 512.
    assert_eq!(rect_of(&a, 2), Some(Rect::new(1153, 25, 766, 510)));
}

#[test]
fn inc_master_moves_windows_into_master_column() {
    let mut wm = wm();
    manage_all(&mut wm, [1, 2, 3]);
    apply(&mut wm, "next-layout");
    apply(&mut wm, "inc-master 1");
    let a = wm.arrange();
    // Two masters split the 1152px column vertically.
    assert_eq!(rect_of(&a, 3), Some(Rect::new(1, 25, 1150, 526)));
    assert_eq!(rect_of(&a, 2), Some(Rect::new(1, 553, 1150, 526)));
    apply(&mut wm, "inc-master -5");
    // nmaster 0: everything in one column.
    let a = wm.arrange();
    assert_eq!(rect_of(&a, 3).map(|r| r.w), Some(1918));
}

#[test]
fn layout_messages_ignored_when_focus_floats() {
    let mut wm = wm();
    manage_all(&mut wm, [1, 2]);
    apply(&mut wm, "next-layout");
    wm.float_at(w(2), Rect::new(10, 10, 100, 100));
    let before = wm.arrange();
    apply(&mut wm, "expand");
    assert_eq!(wm.arrange(), before);
}

#[test]
fn view_shift_copy_actions() {
    let mut wm = wm();
    manage_all(&mut wm, [1, 2]);
    apply(&mut wm, "shift 3");
    assert_eq!(windows(&wm), [w(1)]);
    apply(&mut wm, "copy 4");
    apply(&mut wm, "view 3");
    assert_eq!(windows(&wm), [w(2)]);
    apply(&mut wm, "view 4");
    assert_eq!(windows(&wm), [w(1)]);
    // Unknown workspaces are not created by `view`.
    apply(&mut wm, "view nope");
    assert_eq!(wm.stack_set().current_tag(), "4");
    apply(&mut wm, "toggle-ws");
    assert_eq!(wm.stack_set().current_tag(), "3");
}

#[test]
fn remove_workspace_action() {
    let mut wm = wm();
    manage_all(&mut wm, [1]);
    apply(&mut wm, "view 5");
    manage_all(&mut wm, [2]);
    apply(&mut wm, "remove-workspace");
    assert_eq!(wm.stack_set().current_tag(), "1");
    assert_eq!(windows(&wm), [w(2), w(1)]);
    assert!(!wm.bar_model().workspaces.iter().any(|(t, _)| t == "5"));
}

#[test]
fn toggle_bar_changes_tiling_area() {
    let mut wm = wm();
    manage_all(&mut wm, [1]);
    assert_eq!(wm.arrange().screens[0].bar, Some(Rect::new(0, 0, 1920, 24)));
    assert_eq!(rect_of(&wm.arrange(), 1), Some(AREA.inset(1)));
    apply(&mut wm, "toggle-bar");
    assert!(!wm.bar_visible());
    assert_eq!(wm.arrange().screens[0].bar, None);
    assert_eq!(rect_of(&wm.arrange(), 1), Some(SCREEN.inset(1)));
}

#[test]
fn toggle_fullscreen_action() {
    let mut wm = wm();
    manage_all(&mut wm, [1, 2]);
    apply(&mut wm, "toggle-fullscreen");
    assert!(wm.is_fullscreen(w(2)));
    let p = wm.arrange().placement(w(2)).cloned().expect("placed");
    assert_eq!(p.layer, Layer::Fullscreen);
    apply(&mut wm, "toggle-fullscreen");
    assert!(!wm.is_fullscreen(w(2)));
}

#[test]
fn float_move_floats_in_place_and_raises() {
    let mut wm = wm();
    manage_all(&mut wm, [1, 2]);
    apply(&mut wm, "focus-next");
    let tiled = rect_of(&wm.arrange(), 1).expect("placed");
    assert_eq!(
        apply(&mut wm, "float-move"),
        [Effect::StartPointerMove(w(1))]
    );
    assert!(wm.stack_set().is_floating(w(1)));
    // It stays exactly where it was and moved to the top of the stack.
    assert_eq!(rect_of(&wm.arrange(), 1), Some(tiled));
    assert_eq!(windows(&wm)[0], w(1));
    assert_eq!(
        apply(&mut wm, "float-resize"),
        [Effect::StartPointerResize(w(1))]
    );
    wm.set_float_rect(w(1), Rect::new(50, 60, 400, 300));
    assert_eq!(rect_of(&wm.arrange(), 1), Some(Rect::new(50, 60, 400, 300)));
}

#[test]
fn pointer_ops_need_a_window() {
    let mut wm = wm();
    assert_eq!(apply(&mut wm, "float-move").len(), 0);
    manage_all(&mut wm, [1]);
    wm.request_fullscreen(w(1), true);
    assert_eq!(apply(&mut wm, "float-resize").len(), 0);
}

#[test]
fn set_float_rect_ignores_tiled_windows() {
    let mut wm = wm();
    manage_all(&mut wm, [1]);
    wm.set_float_rect(w(1), Rect::new(0, 0, 10, 10));
    assert!(!wm.stack_set().is_floating(w(1)));
}

#[test]
fn every_default_binding_applies_without_panicking() {
    let mut wm = wm2();
    manage_all(&mut wm, [1, 2, 3]);
    let bindings = wm.config().keybindings.clone();
    for binding in bindings {
        wm.apply(binding.action);
        if wm.is_prompt_open() {
            wm.prompt_key(crate::prompt::PromptKey::Escape);
        }
    }
    let pointer = wm.config().pointer_bindings.clone();
    for binding in pointer {
        wm.apply(binding.action);
    }
    let _ = wm.arrange();
    let _ = wm.bar_model();
}
