use super::*;
use crate::{
    help::HelpSection,
    prompt::PromptKey,
};

fn keys(wm: &mut Wm, keys: &[PromptKey]) {
    for &key in keys {
        assert_eq!(wm.prompt_key(key).len(), 0);
    }
}

fn scroll(wm: &Wm) -> usize {
    wm.help_model().expect("open").scroll
}

#[test]
fn help_takes_focus_and_toggles() {
    let mut wm = wm();
    manage_all(&mut wm, [1]);
    assert_eq!(apply(&mut wm, "help").len(), 0);
    assert!(wm.is_help_open());
    assert!(wm.grabs_keyboard());
    let a = wm.arrange();
    assert_eq!(a.focus, Focus::Help);
    assert_eq!(a.help, Some((S0, SCREEN)));
    let model = wm.help_model().expect("open");
    assert_eq!(model.scroll, 0);
    assert_eq!(model.groups.len(), HelpSection::ALL.len());
    // Pressing the binding again closes it.
    apply(&mut wm, "help");
    assert!(!wm.is_help_open());
    assert!(wm.help_model().is_none());
    let a = wm.arrange();
    assert_eq!((a.focus, a.help), (Focus::Window(w(1)), None));
}

#[test]
fn keys_scroll_and_close() {
    let mut wm = wm();
    for close in [PromptKey::Escape, PromptKey::Char('q'), PromptKey::Return] {
        apply(&mut wm, "help");
        wm.set_help_viewport(20, 30);
        keys(&mut wm, &[PromptKey::Char('j'), PromptKey::Down]);
        assert_eq!(scroll(&wm), 2);
        keys(&mut wm, &[PromptKey::PageDown]);
        assert_eq!(scroll(&wm), 21);
        keys(&mut wm, &[PromptKey::Char('k'), PromptKey::PageUp]);
        assert_eq!(scroll(&wm), 1);
        keys(&mut wm, &[close]);
        assert!(!wm.is_help_open(), "{close:?}");
        assert!(!wm.grabs_keyboard());
    }
    // Reopening starts at the top; a smaller max clamps.
    apply(&mut wm, "help");
    assert_eq!(scroll(&wm), 0);
    wm.set_help_viewport(20, 30);
    keys(&mut wm, &[PromptKey::End]);
    assert_eq!(scroll(&wm), 30);
    wm.set_help_viewport(40, 10);
    assert_eq!(scroll(&wm), 10);
    // A click closes it.
    wm.close_help();
    assert!(!wm.is_help_open());
    // Setting a viewport while closed does nothing.
    wm.set_help_viewport(1, 1);
    assert!(wm.help_model().is_none());
}

#[test]
fn help_prompt_and_picker_are_exclusive() {
    let mut wm = wm();
    apply(&mut wm, "help");
    apply(&mut wm, "prompt view");
    assert!(wm.is_prompt_open());
    assert!(!wm.is_help_open());
    apply(&mut wm, "help");
    assert!(!wm.is_prompt_open());
    assert_eq!(wm.arrange().focus, Focus::Help);
    apply(&mut wm, "pick-layout");
    assert!(wm.is_layout_picker_open());
    assert!(!wm.is_help_open());
    assert_eq!(wm.arrange().help, None);
    apply(&mut wm, "help");
    assert!(!wm.is_layout_picker_open());
    assert_eq!(wm.arrange().layout_picker, None);
    // Keys go to the help overlay: `j` neither types nor picks.
    keys(&mut wm, &[PromptKey::Char('j'), PromptKey::Return]);
    assert!(!wm.grabs_keyboard());
    assert_eq!(
        wm.stack_set().current_workspace().layout.name(),
        "Tall Right"
    );
}

#[test]
fn pointer_does_not_switch_screens_while_open() {
    let mut wm = wm2();
    apply(&mut wm, "help");
    wm.focus_screen_at(1920 + 10, 10);
    assert_eq!(wm.stack_set().current_tag(), "1");
    assert_eq!(wm.arrange().help.map(|(s, _)| s), Some(S0));
    keys(&mut wm, &[PromptKey::Escape]);
    wm.focus_screen_at(1920 + 10, 10);
    assert_eq!(wm.stack_set().current_tag(), "2");
}

#[test]
fn reflects_the_reloaded_config() {
    let mut wm = wm();
    apply(&mut wm, "help");
    let config = Config::from_toml(
        r#"
        modifier = "Alt"
        [keybindings]
        "Mod+b" = "none"
        "#,
    )
    .expect("valid");
    wm.reconfigure(config);
    let model = wm.help_model().expect("still open");
    let combos: Vec<&str> = model
        .groups
        .iter()
        .flat_map(|g| g.entries.iter().map(|e| e.combo.as_str()))
        .collect();
    assert!(combos.contains(&"Alt+Shift+slash"));
    assert!(!combos.contains(&"Alt+b"));
}
