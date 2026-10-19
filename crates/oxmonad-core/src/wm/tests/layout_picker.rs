use super::*;
use crate::{
    layout::Layout,
    prompt::PromptKey,
};

fn open(wm: &mut Wm) {
    assert_eq!(apply(wm, "pick-layout").len(), 0);
    assert!(wm.is_layout_picker_open());
    assert!(wm.grabs_keyboard());
}

fn keys(wm: &mut Wm, keys: &[PromptKey]) {
    for &key in keys {
        assert_eq!(wm.prompt_key(key).len(), 0);
    }
}

fn layout(wm: &Wm) -> String {
    wm.stack_set().current_workspace().layout.name()
}

fn selected(wm: &Wm) -> usize {
    wm.layout_picker_model().expect("open").selected
}

#[test]
fn picker_takes_focus_and_lists_the_ring() {
    let mut wm = wm();
    manage_all(&mut wm, [1]);
    apply(&mut wm, "next-layout");
    open(&mut wm);
    let a = wm.arrange();
    assert_eq!(a.focus, Focus::LayoutPicker);
    assert_eq!(a.layout_picker, Some((S0, SCREEN)));
    assert_eq!(a.prompt, None);
    let model = wm.layout_picker_model().expect("open");
    let ring = LayoutRing::standard(Config::defaults().tabs.height);
    let names: Vec<String> = ring.layouts().iter().map(Layout::name).collect();
    let entry_names: Vec<String> = model.entries.iter().map(|e| e.name.clone()).collect();
    assert_eq!(entry_names, names);
    assert_eq!((model.selected, model.active), (1, 1));
    assert_eq!(model.tag, "1");
    // The previews are the docs pictures' arrangements.
    for (entry, layout) in model.entries.iter().zip(ring.layouts()) {
        assert_eq!(entry.preview, layout.preview());
    }
    assert!(model.entries[4].preview.tabs.is_some());
}

#[test]
fn keys_cycle_wrap_and_apply() {
    let mut wm = wm();
    open(&mut wm);
    keys(&mut wm, &[PromptKey::Char('j'), PromptKey::Char('j')]);
    assert_eq!(selected(&wm), 2);
    keys(&mut wm, &[PromptKey::Char('k')]);
    assert_eq!(selected(&wm), 1);
    keys(&mut wm, &[PromptKey::Up, PromptKey::Up]);
    assert_eq!(selected(&wm), 10);
    // Nothing changes until Return.
    assert_eq!(layout(&wm), "Tall Right");
    keys(&mut wm, &[PromptKey::Return]);
    assert!(!wm.is_layout_picker_open());
    assert_eq!(layout(&wm), "Corner Right");
    assert_eq!(wm.arrange().focus, Focus::None);
}

#[test]
fn escape_cancels() {
    let mut wm = wm();
    manage_all(&mut wm, [1]);
    open(&mut wm);
    keys(&mut wm, &[PromptKey::Down, PromptKey::Escape]);
    assert!(!wm.is_layout_picker_open());
    assert!(wm.layout_picker_model().is_none());
    assert_eq!(layout(&wm), "Tall Right");
    assert_eq!(wm.arrange().focus, Focus::Window(w(1)));
}

#[test]
fn pick_layout_action_toggles() {
    let mut wm = wm();
    open(&mut wm);
    apply(&mut wm, "pick-layout");
    assert!(!wm.grabs_keyboard());
    assert_eq!(layout(&wm), "Tall Right");
}

#[test]
fn click_applies_and_keeps_layout_state() {
    let mut wm = wm();
    manage_all(&mut wm, [1, 2]);
    // Shrink "Tall" (index 1), then move away from it.
    apply(&mut wm, "next-layout");
    apply(&mut wm, "shrink");
    let shrunk = wm.arrange().windows;
    apply(&mut wm, "next-layout");
    open(&mut wm);
    wm.pick_layout(1);
    assert!(!wm.is_layout_picker_open());
    assert_eq!(layout(&wm), "Tall");
    assert_eq!(wm.arrange().windows, shrunk);
    // A click with the picker closed does nothing.
    wm.pick_layout(3);
    assert_eq!(layout(&wm), "Tall");
    // Out of range: closes, keeps the layout.
    open(&mut wm);
    wm.pick_layout(99);
    assert!(!wm.is_layout_picker_open());
    assert_eq!(layout(&wm), "Tall");
}

#[test]
fn digits_apply_directly() {
    let mut wm = wm();
    open(&mut wm);
    keys(&mut wm, &[PromptKey::Char('4')]);
    assert!(!wm.is_layout_picker_open());
    assert_eq!(layout(&wm), "Full");
}

#[test]
fn prompt_and_picker_are_exclusive() {
    let mut wm = wm();
    open(&mut wm);
    apply(&mut wm, "prompt view");
    assert!(wm.is_prompt_open());
    assert!(!wm.is_layout_picker_open());
    assert_eq!(wm.arrange().focus, Focus::Prompt);
    assert_eq!(wm.arrange().layout_picker, None);
    open(&mut wm);
    assert!(!wm.is_prompt_open());
    assert_eq!(wm.arrange().focus, Focus::LayoutPicker);
    // Keys go to the picker, not a prompt.
    keys(&mut wm, &[PromptKey::Char('j'), PromptKey::Return]);
    assert_eq!(layout(&wm), "Tall");
    assert_eq!(wm.stack_set().current_tag(), "1");
}

#[test]
fn picker_stays_on_its_workspace() {
    let mut wm = wm2();
    open(&mut wm);
    // Focus moves to the other screen (e.g. a bar click there).
    apply(&mut wm, "focus-screen next");
    assert_eq!(wm.stack_set().current_tag(), "2");
    assert_eq!(wm.arrange().layout_picker.map(|(s, _)| s), Some(S0));
    // The pointer does not switch screens while the picker is open.
    wm.focus_screen_at(10, 10);
    assert_eq!(wm.stack_set().current_tag(), "2");
    keys(&mut wm, &[PromptKey::Char('j'), PromptKey::Return]);
    assert_eq!(
        wm.stack_set().workspace("1").map(|w| w.layout.name()),
        Some("Tall".into())
    );
    assert_eq!(layout(&wm), "Tall Right");
}
