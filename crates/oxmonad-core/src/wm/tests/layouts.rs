//! The configured layout ring (`layouts`): order, `reset-layout`, the
//! picker and reloads.

use super::*;
use crate::{
    layout::{
        Layout,
        LayoutMessage,
    },
    prompt::PromptKey,
};

/// The default config with `layouts` set to `names`.
fn config(names: &[&str]) -> Config {
    let list = names
        .iter()
        .map(|name| format!("\"{name}\""))
        .collect::<Vec<_>>()
        .join(", ");
    Config::from_toml(&format!("layouts = [{list}]")).expect("config loads")
}

/// A `Wm` with `layouts` set to `names` and one screen.
fn wm_with(names: &[&str]) -> Wm {
    let mut wm = Wm::new(config(names));
    wm.add_output(O0, "DP-1", SCREEN);
    wm
}

fn ring_names(workspace: &Workspace) -> Vec<String> {
    workspace
        .layout
        .layouts()
        .iter()
        .map(Layout::name)
        .collect()
}

fn layout(wm: &Wm) -> String {
    wm.stack_set().current_workspace().layout.name()
}

#[test]
fn every_workspace_starts_with_the_first_configured_layout() {
    let mut wm = wm_with(&["grid", "Full", "Tall"]);
    for workspace in wm.stack_set().workspaces() {
        assert_eq!(ring_names(workspace), ["Grid", "Full", "Tall"]);
        assert_eq!(workspace.layout.name(), "Grid");
    }
    // Workspaces created later use the configured ring too.
    wm.stack_set.add_workspace("new");
    let new = wm.stack_set().workspace("new").expect("added");
    assert_eq!(ring_names(new), ["Grid", "Full", "Tall"]);
}

#[test]
fn next_layout_and_reset_layout_follow_the_configured_ring() {
    let mut wm = wm_with(&["Corner", "Tabs"]);
    manage_all(&mut wm, [1, 2]);
    assert_eq!(layout(&wm), "Corner");
    apply(&mut wm, "next-layout");
    assert_eq!(layout(&wm), "Tabs");
    apply(&mut wm, "next-layout");
    assert_eq!(layout(&wm), "Corner");
    apply(&mut wm, "next-layout");
    apply(&mut wm, "reset-layout");
    assert_eq!(layout(&wm), "Corner");
    // The configured tab height still applies.
    apply(&mut wm, "next-layout");
    let strip = wm.arrange().screens[0]
        .tab_strip
        .clone()
        .expect("tab strip");
    assert_eq!(strip.rect.h, Config::defaults().tabs.height);
}

#[test]
fn picker_lists_and_digits_pick_in_ring_order() {
    let mut wm = wm_with(&["Wide", "Grid", "Tall Right"]);
    manage_all(&mut wm, [1]);
    apply(&mut wm, "pick-layout");
    let model = wm.layout_picker_model().expect("open");
    let names: Vec<String> = model.entries.iter().map(|e| e.name.clone()).collect();
    assert_eq!(names, ["Wide", "Grid", "Tall Right"]);
    wm.prompt_key(PromptKey::Char('3'));
    assert!(!wm.is_layout_picker_open());
    assert_eq!(layout(&wm), "Tall Right");
    // Digits past the ring's end do nothing.
    apply(&mut wm, "pick-layout");
    wm.prompt_key(PromptKey::Char('4'));
    assert_eq!(layout(&wm), "Tall Right");
}

#[test]
fn reload_with_the_same_ring_keeps_layout_state() {
    let mut wm = wm();
    manage_all(&mut wm, [1, 2]);
    apply(&mut wm, "next-layout");
    wm.stack_set.current_workspace_mut().layout.handle(
        LayoutMessage::Expand,
        crate::layout::LayoutContext::default(),
    );
    let before = wm.stack_set().current_workspace().layout.clone();
    assert_ne!(before, LayoutRing::standard(Config::defaults().tabs.height));
    wm.reconfigure(Config::defaults());
    assert_eq!(wm.stack_set().current_workspace().layout, before);
}

#[test]
fn reload_with_a_new_order_keeps_the_active_layout() {
    let mut wm = wm();
    manage_all(&mut wm, [1]);
    apply(&mut wm, "next-layout");
    assert_eq!(layout(&wm), "Tall");
    wm.reconfigure(config(&["Full", "Tall", "Grid"]));
    let current = wm.stack_set().current_workspace();
    assert_eq!(ring_names(current), ["Full", "Tall", "Grid"]);
    assert_eq!(layout(&wm), "Tall");
    // A hidden workspace whose active layout ("Tall Right") is no longer
    // in the ring starts over at the first configured layout.
    let hidden = wm.stack_set().workspace("2").expect("exists");
    assert_eq!(ring_names(hidden), ["Full", "Tall", "Grid"]);
    assert_eq!(hidden.layout.name(), "Full");
    apply(&mut wm, "next-layout");
    assert_eq!(layout(&wm), "Grid");
    apply(&mut wm, "reset-layout");
    assert_eq!(layout(&wm), "Full");
}

#[test]
fn reload_closes_the_layout_picker() {
    let mut wm = wm();
    apply(&mut wm, "pick-layout");
    assert!(wm.is_layout_picker_open());
    wm.reconfigure(config(&["Grid"]));
    assert!(!wm.is_layout_picker_open());
    assert_eq!(layout(&wm), "Grid");
}
