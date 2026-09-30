//! View models for the WM-drawn surfaces (bar, prompt, layout picker and
//! help overlay; tab strips are part of the [`super::Arrangement`]).

use super::Wm;
use crate::{
    help::{
        HelpGroup,
        help_groups,
    },
    layout::LayoutOutput,
    prompt::PromptKind,
    stack_set::SCRATCHPAD_TAG,
};

/// How a workspace appears in the bar (xmobar's `PP` categories).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WorkspaceState {
    /// Shown on the current screen (`ppCurrent`).
    Current,
    /// Shown on another screen (`ppVisible`).
    Visible,
    /// Hidden, with windows (`ppHidden`).
    HiddenNonEmpty,
    /// Hidden and empty (`ppHiddenNoWindows`).
    HiddenEmpty,
}

/// What the bar shows (xmonad's `dynamicLogString` with the default
/// `ppOrder`: workspaces, layout, title).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BarModel {
    /// Workspaces in bar order (configured tags in configuration order,
    /// then the others sorted by tag, like `getSortByIndex`), without the
    /// scratchpad workspace.
    pub workspaces: Vec<(String, WorkspaceState)>,
    /// Description of the current workspace's layout.
    pub layout: String,
    /// Title of the focused window, shortened like xmonad's `shorten`.
    pub title: String,
}

/// What the prompt shows.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PromptModel {
    /// What the prompt does.
    pub kind: PromptKind,
    /// The label in front of the input.
    pub label: String,
    /// The typed text.
    pub input: String,
    /// Cursor position in characters.
    pub cursor: usize,
    /// Completions matching the input.
    pub completions: Vec<String>,
    /// Index of the highlighted completion.
    pub selected: Option<usize>,
}

/// One layout as the layout picker shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayoutPreview {
    /// The layout's name.
    pub name: String,
    /// What it does with four windows in a 16:9 area (see
    /// [`crate::layout::Layout::preview`]).
    pub preview: LayoutOutput,
}

/// What the layout picker shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayoutPickerModel {
    /// The workspace the picker chooses a layout for.
    pub tag: String,
    /// Every layout of the workspace's ring, in ring order.
    pub entries: Vec<LayoutPreview>,
    /// Index of the selected entry.
    pub selected: usize,
    /// Index of the workspace's active layout.
    pub active: usize,
}

/// What the help overlay shows.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct HelpModel {
    /// Every active binding, grouped (see [`help_groups`]).
    pub groups: Vec<HelpGroup>,
    /// The first visible row.
    pub scroll: usize,
    /// The largest scroll offset the renderer last reported.
    pub max_scroll: usize,
}

impl Wm {
    /// The bar's contents.
    #[must_use]
    pub fn bar_model(&self) -> BarModel {
        let ss = &self.stack_set;
        let current = ss.current_tag();
        let workspaces = self
            .bar_tags()
            .into_iter()
            .map(|tag| {
                let state = if tag == current {
                    WorkspaceState::Current
                } else if ss.screen_of_tag(&tag).is_some() {
                    WorkspaceState::Visible
                } else if ss.workspace(&tag).is_some_and(|w| !w.is_empty()) {
                    WorkspaceState::HiddenNonEmpty
                } else {
                    WorkspaceState::HiddenEmpty
                };
                (tag, state)
            })
            .collect();
        let title = ss.peek().map(|w| self.title(w)).unwrap_or_default();
        BarModel {
            workspaces,
            layout: ss.current_workspace().layout.name(),
            title: match self.config.bar.title_max_len {
                Some(max) => shorten(&title, max),
                None => title,
            },
        }
    }

    /// The prompt's contents, if it is open.
    #[must_use]
    pub fn prompt_model(&self) -> Option<PromptModel> {
        let prompt = self.prompt.as_ref()?;
        Some(PromptModel {
            kind: prompt.kind(),
            label: prompt.kind().label().to_owned(),
            input: prompt.input().to_owned(),
            cursor: prompt.cursor(),
            completions: prompt
                .completions()
                .into_iter()
                .map(str::to_owned)
                .collect(),
            selected: prompt.selected(),
        })
    }

    /// The layout picker's contents, if it is open.
    #[must_use]
    pub fn layout_picker_model(&self) -> Option<LayoutPickerModel> {
        let picker = self.picker.as_ref()?;
        let ring = &self.stack_set.workspace(picker.tag())?.layout;
        Some(LayoutPickerModel {
            tag: picker.tag().to_owned(),
            entries: ring
                .layouts()
                .iter()
                .map(|layout| LayoutPreview {
                    name: layout.name(),
                    preview: layout.preview(),
                })
                .collect(),
            selected: picker.selected(),
            active: ring.index(),
        })
    }

    /// The help overlay's contents, if it is open.
    #[must_use]
    pub fn help_model(&self) -> Option<HelpModel> {
        let help = self.help.as_ref()?;
        Some(HelpModel {
            groups: help_groups(&self.config),
            scroll: help.scroll(),
            max_scroll: help.max_scroll(),
        })
    }

    /// Workspace tags in bar order, without the scratchpad workspace.
    pub(super) fn bar_tags(&self) -> Vec<String> {
        let configured = &self.config.workspaces;
        let mut tags: Vec<String> = self
            .stack_set
            .workspaces()
            .map(|w| w.tag.clone())
            .filter(|tag| tag != SCRATCHPAD_TAG)
            .collect();
        tags.sort_by(|a, b| {
            let index = |tag: &String| configured.iter().position(|c| c == tag);
            match (index(a), index(b)) {
                (Some(x), Some(y)) => x.cmp(&y),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => a.cmp(b),
            }
        });
        tags
    }
}

/// xmonad's `shorten n`: strings of `n` or more characters are cut to
/// `n - 3` characters followed by `"..."`.
#[must_use]
pub fn shorten(text: &str, max: usize) -> String {
    const END: &str = "...";
    if text.chars().count() < max {
        return text.to_owned();
    }
    let keep = max.saturating_sub(END.len());
    text.chars().take(keep).chain(END.chars()).collect()
}
