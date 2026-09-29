//! Pure prompt state machine, modelled on `XMonad.Prompt` as used by
//! `XMonad.Actions.DynamicWorkspaces`.

/// What a submitted workspace prompt does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PromptKind {
    /// View the named workspace, creating it if needed (`selectWorkspace`).
    ViewWorkspace,
    /// Move the focused window to the named workspace, creating it if
    /// needed (`withWorkspace … W.shift`).
    MoveToWorkspace,
    /// Copy the focused window to the named workspace, creating it if
    /// needed (`withWorkspace … copy`).
    CopyToWorkspace,
    /// Rename the current workspace (`renameWorkspace`).
    RenameWorkspace,
}

impl PromptKind {
    /// Every kind, in declaration order.
    pub const ALL: [Self; 4] = [
        Self::ViewWorkspace,
        Self::MoveToWorkspace,
        Self::CopyToWorkspace,
        Self::RenameWorkspace,
    ];

    /// The argument used in the `prompt <kind>` action command.
    #[must_use]
    pub const fn command(self) -> &'static str {
        match self {
            Self::ViewWorkspace => "view",
            Self::MoveToWorkspace => "move",
            Self::CopyToWorkspace => "copy",
            Self::RenameWorkspace => "rename",
        }
    }

    /// Parses the argument of the `prompt <kind>` action command.
    #[must_use]
    pub fn from_command(command: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.command() == command)
    }

    /// The label shown in front of the input.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::ViewWorkspace => "Workspace: ",
            Self::MoveToWorkspace => "Move to workspace: ",
            Self::CopyToWorkspace => "Copy to workspace: ",
            Self::RenameWorkspace => "Rename workspace: ",
        }
    }
}

/// An abstract key press fed into an open prompt. The binary translates
/// keysyms into these.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PromptKey {
    /// Insert a character at the cursor.
    Char(char),
    /// Delete the character before the cursor.
    Backspace,
    /// Delete the character under the cursor.
    Delete,
    /// Highlight the next completion.
    Tab,
    /// Highlight the previous completion.
    BackTab,
    /// Confirm.
    Return,
    /// Cancel.
    Escape,
    /// Move the cursor one character left.
    Left,
    /// Move the cursor one character right.
    Right,
    /// Move the cursor to the start.
    Home,
    /// Move the cursor to the end.
    End,
    /// Delete the word before the cursor (`Ctrl+w`).
    DeleteWord,
    /// Delete from the cursor to the end (`Ctrl+k`).
    KillToEnd,
    /// Delete from the start to the cursor (`Ctrl+u`).
    KillToStart,
}

/// The result of feeding one key into a [`Prompt`].
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PromptOutcome {
    /// The prompt stays open.
    Continue,
    /// The prompt was closed without a value.
    Cancel,
    /// The prompt was confirmed with this (trimmed, non-empty) value.
    Submit(String),
}

/// An open prompt: typed input, cursor and completion highlight.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prompt {
    kind: PromptKind,
    input: String,
    /// Cursor position as a char index into `input`.
    cursor: usize,
    options: Vec<String>,
    /// Index into [`Prompt::completions`] of the highlighted entry.
    selected: Option<usize>,
}

impl Prompt {
    /// Opens a prompt of `kind` completing over `options`.
    #[must_use]
    pub fn new(kind: PromptKind, options: Vec<String>) -> Self {
        Self {
            kind,
            input: String::new(),
            cursor: 0,
            options,
            selected: None,
        }
    }

    /// What the prompt is for.
    #[must_use]
    pub fn kind(&self) -> PromptKind {
        self.kind
    }

    /// The typed text.
    #[must_use]
    pub fn input(&self) -> &str {
        &self.input
    }

    /// The cursor position, as a char index into [`Self::input`].
    #[must_use]
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// The options that start with the typed text (xmonad's default
    /// `isPrefixOf` search predicate), in option order.
    #[must_use]
    pub fn completions(&self) -> Vec<&str> {
        self.options
            .iter()
            .filter(|option| option.starts_with(&self.input))
            .map(String::as_str)
            .collect()
    }

    /// Index into [`Self::completions`] of the highlighted completion.
    #[must_use]
    pub fn selected(&self) -> Option<usize> {
        self.selected
    }

    /// The value `Return` would submit: the highlighted completion, else
    /// the typed text.
    #[must_use]
    pub fn value(&self) -> String {
        self.selected
            .and_then(|index| self.completions().get(index).map(|c| (*c).to_owned()))
            .unwrap_or_else(|| self.input.clone())
    }

    /// Feeds one key into the prompt.
    pub fn handle_key(&mut self, key: PromptKey) -> PromptOutcome {
        match key {
            PromptKey::Return => {
                let value = self.value();
                let value = value.trim();
                return if value.is_empty() {
                    PromptOutcome::Cancel
                } else {
                    PromptOutcome::Submit(value.to_owned())
                };
            }
            PromptKey::Escape => return PromptOutcome::Cancel,
            PromptKey::Tab => self.cycle(true),
            PromptKey::BackTab => self.cycle(false),
            PromptKey::Left => self.cursor = self.cursor.saturating_sub(1),
            PromptKey::Right => self.cursor = (self.cursor + 1).min(self.len()),
            PromptKey::Home => self.cursor = 0,
            PromptKey::End => self.cursor = self.len(),
            PromptKey::Char(c) => {
                if !c.is_control() {
                    let at = self.byte_index(self.cursor);
                    self.input.insert(at, c);
                    self.cursor += 1;
                    self.selected = None;
                }
            }
            PromptKey::Backspace => {
                if self.cursor > 0 {
                    self.remove_chars(self.cursor - 1, self.cursor);
                }
            }
            PromptKey::Delete => {
                if self.cursor < self.len() {
                    self.remove_chars(self.cursor, self.cursor + 1);
                }
            }
            PromptKey::DeleteWord => {
                let chars: Vec<char> = self.input.chars().collect();
                let mut start = self.cursor;
                while start > 0 && chars[start - 1].is_whitespace() {
                    start -= 1;
                }
                while start > 0 && !chars[start - 1].is_whitespace() {
                    start -= 1;
                }
                self.remove_chars(start, self.cursor);
            }
            PromptKey::KillToEnd => self.remove_chars(self.cursor, self.len()),
            PromptKey::KillToStart => self.remove_chars(0, self.cursor),
        }
        PromptOutcome::Continue
    }

    fn len(&self) -> usize {
        self.input.chars().count()
    }

    fn byte_index(&self, char_index: usize) -> usize {
        self.input
            .char_indices()
            .nth(char_index)
            .map_or(self.input.len(), |(i, _)| i)
    }

    /// Removes the chars in `start..end`, leaving the cursor at `start`.
    fn remove_chars(&mut self, start: usize, end: usize) {
        let (from, to) = (self.byte_index(start), self.byte_index(end));
        self.input.replace_range(from..to, "");
        self.cursor = start;
        self.selected = None;
    }

    fn cycle(&mut self, forward: bool) {
        let count = self.completions().len();
        if count == 0 {
            self.selected = None;
            return;
        }
        self.selected = Some(match (self.selected, forward) {
            (None, true) => 0,
            (None, false) => count - 1,
            (Some(i), true) => (i + 1) % count,
            (Some(i), false) => (i + count - 1) % count,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prompt() -> Prompt {
        Prompt::new(
            PromptKind::ViewWorkspace,
            ["1", "2", "web", "work", "mail"].map(String::from).to_vec(),
        )
    }

    fn typed(p: &mut Prompt, text: &str) {
        for c in text.chars() {
            assert_eq!(p.handle_key(PromptKey::Char(c)), PromptOutcome::Continue);
        }
    }

    #[test]
    fn prefix_completion() {
        let mut p = prompt();
        assert_eq!(p.completions().len(), 5);
        typed(&mut p, "w");
        assert_eq!(p.completions(), ["web", "work"]);
        typed(&mut p, "o");
        assert_eq!(p.completions(), ["work"]);
        typed(&mut p, "x");
        assert_eq!(p.completions(), Vec::<&str>::new());
    }

    #[test]
    fn tab_cycles_and_wraps() {
        let mut p = prompt();
        typed(&mut p, "w");
        p.handle_key(PromptKey::Tab);
        assert_eq!(p.selected(), Some(0));
        assert_eq!(p.value(), "web");
        p.handle_key(PromptKey::Tab);
        assert_eq!(p.value(), "work");
        p.handle_key(PromptKey::Tab);
        assert_eq!(p.value(), "web");
        p.handle_key(PromptKey::BackTab);
        assert_eq!(p.value(), "work");
        // Input is untouched by cycling.
        assert_eq!(p.input(), "w");
    }

    #[test]
    fn backtab_starts_at_last() {
        let mut p = prompt();
        p.handle_key(PromptKey::BackTab);
        assert_eq!(p.value(), "mail");
    }

    #[test]
    fn tab_without_completions_selects_nothing() {
        let mut p = prompt();
        typed(&mut p, "zzz");
        p.handle_key(PromptKey::Tab);
        assert_eq!(p.selected(), None);
        assert_eq!(p.value(), "zzz");
    }

    #[test]
    fn editing_clears_selection() {
        let mut p = prompt();
        p.handle_key(PromptKey::Tab);
        typed(&mut p, "w");
        assert_eq!(p.selected(), None);
        p.handle_key(PromptKey::Tab);
        p.handle_key(PromptKey::Backspace);
        assert_eq!(p.selected(), None);
        assert_eq!(p.input(), "");
    }

    #[test]
    fn return_submits_highlight_or_input() {
        let mut p = prompt();
        typed(&mut p, "we");
        p.handle_key(PromptKey::Tab);
        assert_eq!(
            p.handle_key(PromptKey::Return),
            PromptOutcome::Submit("web".into())
        );
        let mut p = prompt();
        typed(&mut p, "  new ws ");
        assert_eq!(
            p.handle_key(PromptKey::Return),
            PromptOutcome::Submit("new ws".into())
        );
    }

    #[test]
    fn return_on_empty_cancels() {
        let mut p = prompt();
        typed(&mut p, "   ");
        assert_eq!(p.handle_key(PromptKey::Return), PromptOutcome::Cancel);
        assert_eq!(
            prompt().handle_key(PromptKey::Escape),
            PromptOutcome::Cancel
        );
    }

    #[test]
    fn ignores_control_chars() {
        let mut p = prompt();
        p.handle_key(PromptKey::Char('\u{8}'));
        p.handle_key(PromptKey::Char('\n'));
        assert_eq!(p.input(), "");
        assert_eq!(p.cursor(), 0);
    }

    #[test]
    fn cursor_movement_and_mid_insert() {
        let mut p = prompt();
        typed(&mut p, "ac");
        p.handle_key(PromptKey::Left);
        typed(&mut p, "b");
        assert_eq!(p.input(), "abc");
        assert_eq!(p.cursor(), 2);
        p.handle_key(PromptKey::Home);
        assert_eq!(p.cursor(), 0);
        p.handle_key(PromptKey::Left);
        assert_eq!(p.cursor(), 0);
        p.handle_key(PromptKey::End);
        assert_eq!(p.cursor(), 3);
        p.handle_key(PromptKey::Right);
        assert_eq!(p.cursor(), 3);
    }

    #[test]
    fn backspace_and_delete() {
        let mut p = prompt();
        typed(&mut p, "abcd");
        p.handle_key(PromptKey::Left);
        p.handle_key(PromptKey::Left);
        p.handle_key(PromptKey::Backspace);
        assert_eq!(p.input(), "acd");
        assert_eq!(p.cursor(), 1);
        p.handle_key(PromptKey::Delete);
        assert_eq!(p.input(), "ad");
        assert_eq!(p.cursor(), 1);
        p.handle_key(PromptKey::End);
        p.handle_key(PromptKey::Delete);
        assert_eq!(p.input(), "ad");
        p.handle_key(PromptKey::Home);
        p.handle_key(PromptKey::Backspace);
        assert_eq!(p.input(), "ad");
    }

    #[test]
    fn delete_word_and_kills() {
        let mut p = prompt();
        typed(&mut p, "foo bar  ");
        p.handle_key(PromptKey::DeleteWord);
        assert_eq!(p.input(), "foo ");
        assert_eq!(p.cursor(), 4);
        p.handle_key(PromptKey::DeleteWord);
        assert_eq!(p.input(), "");

        let mut p = prompt();
        typed(&mut p, "hello world");
        for _ in 0..5 {
            p.handle_key(PromptKey::Left);
        }
        p.handle_key(PromptKey::KillToEnd);
        assert_eq!(p.input(), "hello ");
        p.handle_key(PromptKey::Left);
        p.handle_key(PromptKey::KillToStart);
        assert_eq!(p.input(), " ");
        assert_eq!(p.cursor(), 0);
    }

    #[test]
    fn unicode_safe() {
        let mut p = prompt();
        typed(&mut p, "äöü");
        p.handle_key(PromptKey::Left);
        p.handle_key(PromptKey::Backspace);
        assert_eq!(p.input(), "äü");
        typed(&mut p, "🦀");
        assert_eq!(p.input(), "ä🦀ü");
        assert_eq!(p.cursor(), 2);
        p.handle_key(PromptKey::DeleteWord);
        assert_eq!(p.input(), "ü");
    }

    #[test]
    fn kind_is_kept() {
        let p = Prompt::new(PromptKind::RenameWorkspace, Vec::new());
        assert_eq!(p.kind(), PromptKind::RenameWorkspace);
        assert_eq!(p.kind().label(), "Rename workspace: ");
    }
}
