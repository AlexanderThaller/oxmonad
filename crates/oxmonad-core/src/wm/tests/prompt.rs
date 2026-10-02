use super::*;
use crate::prompt::{
    PromptKey,
    PromptKind,
};

fn type_text(wm: &mut Wm, text: &str) {
    for c in text.chars() {
        wm.prompt_key(PromptKey::Char(c));
    }
}

fn open(wm: &mut Wm, kind: PromptKind) {
    assert_eq!(wm.apply(Action::Prompt(kind)).len(), 0);
    assert!(wm.is_prompt_open());
}

#[test]
fn prompt_takes_focus_and_exposes_model() {
    let mut wm = wm();
    // Two windows, so smartBorders keeps their borders.
    manage_all(&mut wm, [1, 2]);
    wm.focus_window(w(1));
    open(&mut wm, PromptKind::ViewWorkspace);
    let a = wm.arrange();
    assert_eq!(a.focus, Focus::Prompt);
    // Bottom of the usable area, 28px high.
    assert_eq!(a.prompt, Some((S0, Rect::new(0, 1052, 1920, 28))));
    // The focused window keeps its focused border.
    assert_eq!(
        a.placement(w(1)).and_then(|p| p.border).map(|b| b.color),
        Some(Config::defaults().border.focused)
    );
    type_text(&mut wm, "1");
    let model = wm.prompt_model().expect("open");
    assert_eq!(model.kind, PromptKind::ViewWorkspace);
    assert_eq!(model.label, "Workspace: ");
    assert_eq!(model.input, "1");
    assert_eq!(model.cursor, 1);
    assert_eq!(model.completions, ["1"]);
    assert_eq!(model.selected, None);
}

#[test]
fn escape_cancels() {
    let mut wm = wm();
    open(&mut wm, PromptKind::ViewWorkspace);
    type_text(&mut wm, "5");
    wm.prompt_key(PromptKey::Escape);
    assert!(!wm.is_prompt_open());
    assert!(wm.prompt_model().is_none());
    assert_eq!(wm.stack_set().current_tag(), "1");
    // Keys without a prompt do nothing.
    assert_eq!(wm.prompt_key(PromptKey::Return).len(), 0);
}

#[test]
fn view_prompt_creates_and_views() {
    let mut wm = wm();
    open(&mut wm, PromptKind::ViewWorkspace);
    type_text(&mut wm, "web");
    wm.prompt_key(PromptKey::Return);
    assert!(!wm.is_prompt_open());
    assert_eq!(wm.stack_set().current_tag(), "web");
    // Viewing an existing workspace by Tab completion: the second
    // completion of the empty input is "2".
    open(&mut wm, PromptKind::ViewWorkspace);
    wm.prompt_key(PromptKey::Tab);
    wm.prompt_key(PromptKey::Tab);
    wm.prompt_key(PromptKey::Return);
    assert_eq!(wm.stack_set().current_tag(), "2");
}

#[test]
fn view_prompt_on_visible_workspace_swaps_screens() {
    let mut wm = wm2();
    open(&mut wm, PromptKind::ViewWorkspace);
    type_text(&mut wm, "2");
    wm.prompt_key(PromptKey::Return);
    assert_eq!(wm.stack_set().screen_of_tag("2"), Some(S0));
    assert_eq!(wm.stack_set().screen_of_tag("1"), Some(S1));
}

#[test]
fn move_and_copy_prompts() {
    let mut wm = wm();
    manage_all(&mut wm, [1, 2]);
    open(&mut wm, PromptKind::MoveToWorkspace);
    type_text(&mut wm, "mail");
    wm.prompt_key(PromptKey::Return);
    assert_eq!(windows(&wm), [w(1)]);
    assert_eq!(
        wm.stack_set().workspace("mail").map(Workspace::windows),
        Some(vec![w(2)])
    );
    open(&mut wm, PromptKind::CopyToWorkspace);
    type_text(&mut wm, "3");
    wm.prompt_key(PromptKey::Return);
    assert_eq!(windows(&wm), [w(1)]);
    assert_eq!(
        wm.stack_set().workspace("3").map(Workspace::windows),
        Some(vec![w(1)])
    );
}

#[test]
fn rename_prompt() {
    let mut wm = wm();
    open(&mut wm, PromptKind::RenameWorkspace);
    type_text(&mut wm, "code");
    wm.prompt_key(PromptKey::Return);
    assert_eq!(wm.stack_set().current_tag(), "code");
    let tags: Vec<String> = wm
        .bar_model()
        .workspaces
        .into_iter()
        .map(|(t, _)| t)
        .collect();
    assert_eq!(tags.last().map(String::as_str), Some("code"));
    assert!(!tags.contains(&"1".to_owned()));
}

#[test]
fn empty_submission_cancels() {
    let mut wm = wm();
    open(&mut wm, PromptKind::ViewWorkspace);
    type_text(&mut wm, "   ");
    wm.prompt_key(PromptKey::Return);
    assert!(!wm.is_prompt_open());
    assert_eq!(wm.stack_set().current_tag(), "1");
}

#[test]
fn completions_exclude_scratchpad_workspace() {
    let mut wm = wm();
    wm.manage(w(1), info("scratchpad", "term"));
    apply(&mut wm, "toggle-scratchpad term");
    assert!(wm.stack_set().has_tag(crate::stack_set::SCRATCHPAD_TAG));
    open(&mut wm, PromptKind::ViewWorkspace);
    type_text(&mut wm, "N");
    assert_eq!(wm.prompt_model().expect("open").completions.len(), 0);
}
