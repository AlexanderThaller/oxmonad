//! Interactive text prompt, modelled on `XMonad.Prompt`.
//!
//! A [`Prompt`] captures all keyboard input while open (see [`crate::input`]),
//! offering prefix-filtered completions over a set of candidate names — here,
//! workspace names. `Tab` cycles through the completions, `Return` submits the
//! current value, `Escape` cancels. The prompt is drawn as a strip along the
//! top of the active monitor using the built-in bar font, with the colors from
//! the user's xmonad `myXPConfig`.

use smithay::{
    backend::renderer::element::solid::SolidColorRenderElement,
    input::keyboard::Keysym,
    utils::{
        Logical,
        Point,
        Rectangle,
        Size,
    },
};

use crate::{
    bar,
    config,
};

/// What the submitted prompt value is used for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptKind {
    /// View the named workspace, creating it if needed (`selectWorkspace`).
    Select,
    /// Move the focused window to the named workspace (`withWorkspace shift`).
    MoveTo,
    /// Copy the focused window to the named workspace (`withWorkspace copy`).
    CopyTo,
    /// Rename the current workspace (`renameWorkspace`).
    Rename,
}

impl PromptKind {
    /// The label shown before the input, like xmonad's prompt prefix.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Select => "Workspace:",
            Self::MoveTo => "Move to:",
            Self::CopyTo => "Copy to:",
            Self::Rename => "Rename to:",
        }
    }
}

/// The result of feeding one key press into the prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptOutcome {
    /// The prompt consumed the key and stays open.
    Continue,
    /// The prompt was cancelled (`Escape`).
    Cancel,
    /// The prompt was submitted with this value (`Return`).
    Submit(String),
}

/// An open prompt: the typed input and the completion state.
#[derive(Debug, Clone)]
pub struct Prompt {
    /// What the submitted value will be used for.
    pub kind: PromptKind,
    /// The text typed so far.
    input: String,
    /// All candidate completions (workspace names, in workspace order).
    options: Vec<String>,
    /// Index into [`Self::completions`] of the Tab-selected entry.
    selected: Option<usize>,
}

impl Prompt {
    /// Opens a new prompt of `kind` completing over `options`.
    #[must_use]
    pub fn new(kind: PromptKind, options: Vec<String>) -> Self {
        Self {
            kind,
            input: String::new(),
            options,
            selected: None,
        }
    }

    /// The typed input.
    #[must_use]
    pub fn input(&self) -> &str {
        &self.input
    }

    /// The candidates whose names start with the typed input (xmonad's
    /// default `searchPredicate = isPrefixOf`).
    #[must_use]
    pub fn completions(&self) -> Vec<&str> {
        self.options
            .iter()
            .filter(|option| option.starts_with(&self.input))
            .map(String::as_str)
            .collect()
    }

    /// Index of the Tab-selected completion, if any.
    #[must_use]
    pub fn selected(&self) -> Option<usize> {
        self.selected
    }

    /// The value a `Return` would submit: the Tab-selected completion if one
    /// is highlighted, otherwise the raw input.
    #[must_use]
    pub fn value(&self) -> String {
        if let Some(selected) = self.selected
            && let Some(completion) = self.completions().get(selected)
        {
            return (*completion).to_string();
        }
        self.input.clone()
    }

    /// Feeds one key press into the prompt.
    ///
    /// `ch` is the character produced by the key under the current modifiers
    /// (so shifted letters arrive uppercase), if it produces one.
    pub fn handle_key(&mut self, keysym: Keysym, ch: Option<char>) -> PromptOutcome {
        match keysym {
            Keysym::Return | Keysym::KP_Enter => {
                return PromptOutcome::Submit(self.value());
            }
            Keysym::Escape => return PromptOutcome::Cancel,
            Keysym::Tab => self.cycle(1),
            Keysym::ISO_Left_Tab => self.cycle(-1),
            Keysym::BackSpace => {
                self.input.pop();
                self.selected = None;
            }
            _ => {
                if let Some(ch) = ch.filter(|ch| !ch.is_control()) {
                    self.input.push(ch);
                    self.selected = None;
                }
            }
        }
        PromptOutcome::Continue
    }

    /// Moves the completion highlight by `step`, wrapping.
    fn cycle(&mut self, step: isize) {
        let count = self.completions().len();
        if count == 0 {
            return;
        }
        let len = isize::try_from(count).unwrap_or(1);
        let current = self
            .selected
            .and_then(|s| isize::try_from(s).ok())
            .unwrap_or(if step > 0 { -1 } else { 0 });
        let next = (current + step).rem_euclid(len);
        self.selected = usize::try_from(next).ok();
    }
}

/// Pixels of padding between the prompt's input area and the completions, and
/// between completion entries.
const COMPLETION_GAP: i32 = 3 * bar::glyph_advance();

/// Builds the render elements for `prompt` as a strip along the top of
/// `output_geo`, front-to-back.
#[must_use]
pub fn prompt_elements(
    prompt: &Prompt,
    output_geo: Rectangle<i32, Logical>,
    scale: f64,
) -> Vec<SolidColorRenderElement> {
    let height = config::BAR_HEIGHT.clamp(0, output_geo.size.h);
    if height == 0 {
        return Vec::new();
    }

    let strip = Rectangle::new(output_geo.loc, Size::from((output_geo.size.w, height)));
    let advance = bar::glyph_advance();
    let y = strip.loc.y + (height - bar::GLYPH_H * bar::GLYPH_SCALE) / 2;
    let right_edge = strip.loc.x + strip.size.w - bar::TEXT_X;
    let mut x = strip.loc.x + bar::TEXT_X;
    let mut elements = Vec::new();

    // "Label: input_" — the trailing underscore is the cursor.
    let text = format!("{} {}_", prompt.kind.label(), prompt.input());
    bar::draw_text(
        &mut elements,
        Point::from((x, y)),
        (right_edge - x).max(0),
        &text,
        config::PROMPT_FG_COLOR,
        scale,
    );
    x += i32::try_from(text.chars().count()).unwrap_or(0) * advance + COMPLETION_GAP;

    // The completions, with the Tab-selected one highlighted.
    let mut highlights = Vec::new();
    for (index, completion) in prompt.completions().iter().enumerate() {
        let chars = i32::try_from(completion.chars().count()).unwrap_or(0);
        if x + chars * advance > right_edge {
            break;
        }
        let selected = prompt.selected() == Some(index);
        let color = if selected {
            config::PROMPT_HL_FG_COLOR
        } else {
            config::PROMPT_FG_COLOR
        };
        if selected {
            // Backing rectangle behind the selected completion.
            highlights.push(bar::solid_element(
                Rectangle::new(
                    Point::from((x - advance / 2, strip.loc.y + 1)),
                    Size::from((chars * advance + advance / 2, height - 2)),
                ),
                config::PROMPT_HL_BG_COLOR,
                scale,
            ));
        }
        bar::draw_text(
            &mut elements,
            Point::from((x, y)),
            (right_edge - x).max(0),
            completion,
            color,
            scale,
        );
        x += chars * advance + COMPLETION_GAP;
    }

    // Front-to-back: text, then the highlight rectangles, a border line under
    // the strip, and finally the background.
    elements.extend(highlights);
    elements.push(bar::solid_element(
        Rectangle::new(
            Point::from((strip.loc.x, strip.loc.y + height - 1)),
            Size::from((strip.size.w, 1)),
        ),
        config::PROMPT_BORDER_COLOR,
        scale,
    ));
    elements.push(bar::solid_element(strip, config::PROMPT_BG_COLOR, scale));

    elements
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace_prompt() -> Prompt {
        Prompt::new(
            PromptKind::Select,
            vec!["1".into(), "12".into(), "2".into(), "web".into()],
        )
    }

    #[test]
    fn typing_filters_completions_by_prefix() {
        let mut prompt = workspace_prompt();
        assert_eq!(prompt.completions(), vec!["1", "12", "2", "web"]);

        prompt.handle_key(Keysym::a, Some('1'));
        assert_eq!(prompt.completions(), vec!["1", "12"]);

        prompt.handle_key(Keysym::a, Some('3'));
        assert!(prompt.completions().is_empty());
    }

    #[test]
    fn backspace_removes_the_last_character() {
        let mut prompt = workspace_prompt();
        prompt.handle_key(Keysym::a, Some('w'));
        prompt.handle_key(Keysym::a, Some('x'));
        prompt.handle_key(Keysym::BackSpace, None);
        assert_eq!(prompt.input(), "w");
        assert_eq!(prompt.completions(), vec!["web"]);
    }

    #[test]
    fn tab_cycles_through_completions_and_return_submits_the_selection() {
        let mut prompt = workspace_prompt();
        prompt.handle_key(Keysym::Tab, None);
        assert_eq!(prompt.value(), "1");
        prompt.handle_key(Keysym::Tab, None);
        assert_eq!(prompt.value(), "12");
        prompt.handle_key(Keysym::ISO_Left_Tab, None);
        assert_eq!(prompt.value(), "1");

        assert_eq!(
            prompt.handle_key(Keysym::Return, Some('\r')),
            PromptOutcome::Submit("1".to_string())
        );
    }

    #[test]
    fn return_submits_the_typed_input_when_nothing_is_selected() {
        let mut prompt = workspace_prompt();
        prompt.handle_key(Keysym::a, Some('n'));
        prompt.handle_key(Keysym::a, Some('e'));
        prompt.handle_key(Keysym::a, Some('w'));
        assert_eq!(
            prompt.handle_key(Keysym::Return, Some('\r')),
            PromptOutcome::Submit("new".to_string())
        );
    }

    #[test]
    fn escape_cancels() {
        let mut prompt = workspace_prompt();
        assert_eq!(
            prompt.handle_key(Keysym::Escape, None),
            PromptOutcome::Cancel
        );
    }

    #[test]
    fn typing_resets_the_tab_selection() {
        let mut prompt = workspace_prompt();
        prompt.handle_key(Keysym::Tab, None);
        assert_eq!(prompt.selected(), Some(0));
        prompt.handle_key(Keysym::a, Some('w'));
        assert_eq!(prompt.selected(), None);
    }

    #[test]
    fn control_characters_are_ignored() {
        let mut prompt = workspace_prompt();
        prompt.handle_key(Keysym::a, Some('\u{7f}'));
        assert_eq!(prompt.input(), "");
    }

    #[test]
    fn snapshot_prompt_with_completions() {
        let mut prompt = workspace_prompt();
        prompt.handle_key(Keysym::Tab, None);
        prompt.handle_key(Keysym::Tab, None);

        let elements = prompt_elements(
            &prompt,
            Rectangle::new(Point::from((0, 0)), Size::from((1000, 1000))),
            1.0,
        );
        crate::snapshot::assert_snapshot("workspace_prompt", 1000, 1000, &elements);
    }
}
