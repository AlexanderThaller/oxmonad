//! Pure layout picker state machine: a grid of a workspace's layouts, one
//! of them selected, driven by the same abstract keys as the prompt.

use crate::prompt::PromptKey;

/// The result of feeding one key into a [`LayoutPicker`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PickerOutcome {
    /// The picker stays open.
    Continue,
    /// The picker was closed without a change.
    Cancel,
    /// The picker was closed choosing the layout at this ring index.
    Apply(usize),
}

/// An open layout picker for the workspace `tag`, whose ring has `count`
/// layouts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayoutPicker {
    tag: String,
    count: usize,
    selected: usize,
}

impl LayoutPicker {
    /// Opens a picker over `count` layouts (at least one) of workspace
    /// `tag`, with the active layout `active` selected.
    #[must_use]
    pub fn new(tag: String, count: usize, active: usize) -> Self {
        let count = count.max(1);
        Self {
            tag,
            count,
            selected: active.min(count - 1),
        }
    }

    /// The workspace the picker chooses a layout for.
    #[must_use]
    pub fn tag(&self) -> &str {
        &self.tag
    }

    /// Ring index of the selected layout.
    #[must_use]
    pub fn selected(&self) -> usize {
        self.selected
    }

    /// Feeds one key into the picker: `j`, `Down`, `Right` and `Tab` select
    /// the next layout, `k`, `Up`, `Left` and `Shift+Tab` the previous one
    /// (both wrapping), `Return` or `space` apply the selection, `1`–`9` and
    /// `0` apply the first to tenth layout, `Escape` cancels.
    pub fn handle_key(&mut self, key: PromptKey) -> PickerOutcome {
        match key {
            PromptKey::Char('j') | PromptKey::Down | PromptKey::Right | PromptKey::Tab => {
                self.selected = (self.selected + 1) % self.count;
            }
            PromptKey::Char('k') | PromptKey::Up | PromptKey::Left | PromptKey::BackTab => {
                self.selected = (self.selected + self.count - 1) % self.count;
            }
            PromptKey::Return | PromptKey::Char(' ') => {
                return PickerOutcome::Apply(self.selected);
            }
            PromptKey::Escape => return PickerOutcome::Cancel,
            PromptKey::Char(digit @ '0'..='9') => {
                // `1` is the first layout, `0` the tenth.
                let index = digit
                    .to_digit(10)
                    .and_then(|d| usize::try_from(d).ok())
                    .map_or(usize::MAX, |d| (d + 9) % 10);
                if index < self.count {
                    self.selected = index;
                    return PickerOutcome::Apply(index);
                }
            }
            PromptKey::Char(_)
            | PromptKey::Backspace
            | PromptKey::Delete
            | PromptKey::Home
            | PromptKey::End
            | PromptKey::DeleteWord
            | PromptKey::KillToEnd
            | PromptKey::KillToStart => {}
        }
        PickerOutcome::Continue
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picker(active: usize) -> LayoutPicker {
        LayoutPicker::new("1".into(), 4, active)
    }

    #[test]
    fn starts_on_the_active_layout() {
        assert_eq!(picker(2).selected(), 2);
        assert_eq!(picker(9).selected(), 3);
        assert_eq!(LayoutPicker::new("x".into(), 0, 0).selected(), 0);
        assert_eq!(picker(0).tag(), "1");
    }

    #[test]
    fn cycles_and_wraps() {
        let mut p = picker(0);
        for (key, expected) in [
            (PromptKey::Char('j'), 1),
            (PromptKey::Down, 2),
            (PromptKey::Right, 3),
            (PromptKey::Tab, 0),
            (PromptKey::Char('k'), 3),
            (PromptKey::Up, 2),
            (PromptKey::Left, 1),
            (PromptKey::BackTab, 0),
            (PromptKey::BackTab, 3),
        ] {
            assert_eq!(p.handle_key(key), PickerOutcome::Continue, "{key:?}");
            assert_eq!(p.selected(), expected, "{key:?}");
        }
    }

    #[test]
    fn return_applies_and_escape_cancels() {
        let mut p = picker(1);
        p.handle_key(PromptKey::Char('j'));
        assert_eq!(p.handle_key(PromptKey::Return), PickerOutcome::Apply(2));
        assert_eq!(p.handle_key(PromptKey::Char(' ')), PickerOutcome::Apply(2));
        assert_eq!(p.handle_key(PromptKey::Escape), PickerOutcome::Cancel);
    }

    #[test]
    fn digits_jump_and_apply() {
        let mut p = picker(0);
        assert_eq!(p.handle_key(PromptKey::Char('3')), PickerOutcome::Apply(2));
        assert_eq!(p.selected(), 2);
        // Only four layouts: 5 and 0 (the tenth) do nothing.
        assert_eq!(p.handle_key(PromptKey::Char('5')), PickerOutcome::Continue);
        assert_eq!(p.handle_key(PromptKey::Char('0')), PickerOutcome::Continue);
        let mut ten = LayoutPicker::new("1".into(), 10, 0);
        assert_eq!(
            ten.handle_key(PromptKey::Char('0')),
            PickerOutcome::Apply(9)
        );
    }

    #[test]
    fn other_keys_are_ignored() {
        let mut p = picker(1);
        for key in [PromptKey::Char('x'), PromptKey::Backspace, PromptKey::Home] {
            assert_eq!(p.handle_key(key), PickerOutcome::Continue);
        }
        assert_eq!(p.selected(), 1);
    }
}
