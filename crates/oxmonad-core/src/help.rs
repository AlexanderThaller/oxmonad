//! The key binding help overlay (xmonad's `Mod+Shift+/`): every active key
//! and pointer binding of a [`Config`], grouped into [`HelpSection`]s, and
//! the scroll state of the open overlay, driven by the same abstract keys
//! as the prompt.

use std::fmt;

use crate::{
    action::Action,
    config::Config,
    prompt::PromptKey,
};

/// A group of bindings in the help overlay, in display order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum HelpSection {
    /// Programs and scratchpads.
    Launch,
    /// Focusing, moving and closing windows.
    Windows,
    /// Layout choice and resizing.
    Layouts,
    /// Viewing, moving to and editing workspaces.
    Workspaces,
    /// Screens and screen splits.
    Screens,
    /// The bar, restarting, quitting and this help.
    Session,
    /// Every pointer binding.
    Mouse,
}

impl HelpSection {
    /// Every section in display order.
    pub const ALL: [Self; 7] = [
        Self::Launch,
        Self::Windows,
        Self::Layouts,
        Self::Workspaces,
        Self::Screens,
        Self::Session,
        Self::Mouse,
    ];

    /// The section's heading.
    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Self::Launch => "Launch",
            Self::Windows => "Windows",
            Self::Layouts => "Layouts",
            Self::Workspaces => "Workspaces",
            Self::Screens => "Screens",
            Self::Session => "Session",
            Self::Mouse => "Mouse",
        }
    }
}

/// One binding as the help overlay shows it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct HelpEntry {
    /// The combination with `Mod` resolved, e.g. `Super+Shift+Return`.
    pub combo: String,
    /// The action in config syntax, e.g. `spawn-terminal`.
    pub action: String,
}

/// A non-empty section of the help overlay.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct HelpGroup {
    /// Which section.
    pub section: HelpSection,
    /// Its bindings.
    pub entries: Vec<HelpEntry>,
}

/// Where a key binding's action goes, and its rank within the section
/// (related actions stay together, in the order of the default config).
fn classify(action: &Action) -> (HelpSection, u8) {
    use HelpSection::{
        Launch,
        Layouts,
        Screens,
        Session,
        Windows,
        Workspaces,
    };
    match action {
        Action::SpawnTerminal => (Launch, 0),
        Action::Spawn(_) => (Launch, 1),
        Action::ToggleScratchpad(_) => (Launch, 2),
        Action::FocusNext => (Windows, 0),
        Action::FocusPrev => (Windows, 1),
        Action::FocusMaster => (Windows, 2),
        Action::SwapNext => (Windows, 3),
        Action::SwapPrev => (Windows, 4),
        Action::SwapMaster => (Windows, 5),
        Action::Sink => (Windows, 6),
        Action::ToggleFullscreen => (Windows, 7),
        Action::FloatMove => (Windows, 8),
        Action::FloatResize => (Windows, 9),
        Action::Close => (Windows, 10),
        Action::NextLayout => (Layouts, 0),
        Action::ResetLayout => (Layouts, 1),
        Action::PickLayout => (Layouts, 2),
        Action::Shrink => (Layouts, 3),
        Action::Expand => (Layouts, 4),
        Action::MirrorShrink => (Layouts, 5),
        Action::MirrorExpand => (Layouts, 6),
        Action::IncMaster(_) => (Layouts, 7),
        Action::View(_) => (Workspaces, 0),
        Action::Shift(_) => (Workspaces, 1),
        Action::Copy(_) => (Workspaces, 2),
        Action::ToggleWs => (Workspaces, 3),
        Action::Prompt(_) => (Workspaces, 4),
        Action::RemoveWorkspace => (Workspaces, 5),
        Action::FocusScreen(_) => (Screens, 0),
        Action::ShiftScreen(_) => (Screens, 1),
        Action::ShiftScreenFollow(_) => (Screens, 2),
        Action::SwapNextScreen => (Screens, 3),
        Action::SplitScreen(_) => (Screens, 4),
        Action::Rescreen => (Screens, 5),
        Action::Help => (Session, 0),
        Action::ToggleBar => (Session, 1),
        Action::Refresh => (Session, 2),
        Action::Reload => (Session, 3),
        Action::Restart => (Session, 4),
        Action::Quit => (Session, 5),
    }
}

/// Every active binding of `config`, grouped: key bindings by what their
/// action does, pointer bindings under [`HelpSection::Mouse`]. Sections are
/// in [`HelpSection::ALL`] order and empty ones are left out; within a
/// section bindings are ordered by action kind, then combination.
#[must_use]
pub fn help_groups(config: &Config) -> Vec<HelpGroup> {
    // Sorted by section, action kind, then combination (modifiers first),
    // so e.g. `view 1`…`view 9` follow each other.
    let mut keyed: Vec<((HelpSection, u8, u8, String), HelpEntry)> = config
        .keybindings
        .iter()
        .map(|binding| {
            let (section, rank) = classify(&binding.action);
            (
                (
                    section,
                    rank,
                    binding.combo.mods.0,
                    binding.combo.key.clone(),
                ),
                entry(&binding.combo, &binding.action),
            )
        })
        .chain(config.pointer_bindings.iter().map(|binding| {
            let rank = classify(&binding.action).1;
            let button = binding.combo.button.to_string();
            (
                (HelpSection::Mouse, rank, binding.combo.mods.0, button),
                entry(&binding.combo, &binding.action),
            )
        }))
        .collect();
    keyed.sort_by(|a, b| a.0.cmp(&b.0));
    let mut groups: Vec<HelpGroup> = Vec::new();
    for ((section, ..), entry) in keyed {
        match groups.last_mut() {
            Some(group) if group.section == section => group.entries.push(entry),
            _ => groups.push(HelpGroup {
                section,
                entries: vec![entry],
            }),
        }
    }
    groups
}

fn entry(combo: &impl fmt::Display, action: &Action) -> HelpEntry {
    HelpEntry {
        combo: combo.to_string(),
        action: action.to_string(),
    }
}

/// The result of feeding one key into the [`Help`] overlay.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HelpOutcome {
    /// The overlay stays open.
    Continue,
    /// The overlay was closed.
    Close,
}

/// The open help overlay: how far it is scrolled, in rows. The renderer
/// reports how many rows fit ([`Self::set_viewport`]); until then nothing
/// scrolls.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Help {
    scroll: usize,
    page: usize,
    max_scroll: usize,
}

impl Help {
    /// The overlay scrolled to the top.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The first visible row.
    #[must_use]
    pub fn scroll(&self) -> usize {
        self.scroll
    }

    /// The largest useful scroll offset (0 when everything fits).
    #[must_use]
    pub fn max_scroll(&self) -> usize {
        self.max_scroll
    }

    /// Records that `page` rows are visible at once and the content scrolls
    /// by at most `max_scroll` rows, clamping the offset.
    pub fn set_viewport(&mut self, page: usize, max_scroll: usize) {
        self.page = page;
        self.max_scroll = max_scroll;
        self.scroll = self.scroll.min(max_scroll);
    }

    /// Feeds one key into the overlay: `j`/`Down` and `k`/`Up` scroll by a
    /// row, `PageDown`/`space` and `PageUp` by a page (keeping one row),
    /// `Home`/`End` jump to either end; `Escape`, `q` and `Return` close it.
    pub fn handle_key(&mut self, key: PromptKey) -> HelpOutcome {
        let page = self.page.saturating_sub(1).max(1);
        let scroll = match key {
            PromptKey::Escape | PromptKey::Return | PromptKey::Char('q') => {
                return HelpOutcome::Close;
            }
            PromptKey::Char('j') | PromptKey::Down => self.scroll.saturating_add(1),
            PromptKey::Char('k') | PromptKey::Up => self.scroll.saturating_sub(1),
            PromptKey::PageDown | PromptKey::Char(' ') => self.scroll.saturating_add(page),
            PromptKey::PageUp => self.scroll.saturating_sub(page),
            PromptKey::Home => 0,
            PromptKey::End => self.max_scroll,
            PromptKey::Char(_)
            | PromptKey::Backspace
            | PromptKey::Delete
            | PromptKey::Tab
            | PromptKey::BackTab
            | PromptKey::Left
            | PromptKey::Right
            | PromptKey::DeleteWord
            | PromptKey::KillToEnd
            | PromptKey::KillToStart => self.scroll,
        };
        self.scroll = scroll.min(self.max_scroll);
        HelpOutcome::Continue
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entries(groups: &[HelpGroup]) -> Vec<(HelpSection, &str, &str)> {
        groups
            .iter()
            .flat_map(|g| {
                g.entries
                    .iter()
                    .map(|e| (g.section, e.combo.as_str(), e.action.as_str()))
            })
            .collect()
    }

    #[test]
    fn every_default_binding_appears_exactly_once() {
        let config = Config::defaults();
        let groups = help_groups(&config);
        let shown = entries(&groups);
        assert_eq!(
            shown.len(),
            config.keybindings.len() + config.pointer_bindings.len()
        );
        for binding in &config.keybindings {
            let (combo, action) = (binding.combo.to_string(), binding.action.to_string());
            let count = shown
                .iter()
                .filter(|(_, c, a)| *c == combo && *a == action)
                .count();
            assert_eq!(count, 1, "{combo} = {action}");
        }
        for binding in &config.pointer_bindings {
            let (combo, action) = (binding.combo.to_string(), binding.action.to_string());
            assert!(
                shown.contains(&(HelpSection::Mouse, combo.as_str(), action.as_str())),
                "{combo}"
            );
        }
    }

    #[test]
    fn default_sections_in_order() {
        let groups = help_groups(&Config::defaults());
        let sections: Vec<HelpSection> = groups.iter().map(|g| g.section).collect();
        assert_eq!(sections, HelpSection::ALL);
        let find = |combo: &str| {
            entries(&groups)
                .into_iter()
                .find(|(_, c, _)| *c == combo)
                .map(|(section, _, action)| (section, action.to_owned()))
        };
        assert_eq!(
            find("Super+Shift+Return"),
            Some((HelpSection::Launch, "spawn-terminal".into()))
        );
        assert_eq!(
            find("Super+j"),
            Some((HelpSection::Windows, "focus-next".into()))
        );
        assert_eq!(
            find("Super+Ctrl+space"),
            Some((HelpSection::Layouts, "pick-layout".into()))
        );
        assert_eq!(
            find("Super+Shift+3"),
            Some((HelpSection::Workspaces, "shift 3".into()))
        );
        assert_eq!(
            find("Super+Right"),
            Some((HelpSection::Screens, "focus-screen next".into()))
        );
        assert_eq!(
            find("Super+Shift+slash"),
            Some((HelpSection::Session, "help".into()))
        );
        assert_eq!(
            find("Super+Button1"),
            Some((HelpSection::Mouse, "float-move".into()))
        );
    }

    #[test]
    fn related_bindings_stay_together() {
        let groups = help_groups(&Config::defaults());
        let workspaces = &groups[3];
        assert_eq!(workspaces.section, HelpSection::Workspaces);
        let actions: Vec<&str> = workspaces
            .entries
            .iter()
            .map(|e| e.action.as_str())
            .collect();
        let expected_views: Vec<String> = (1..=9).map(|n| format!("view {n}")).collect();
        assert_eq!(actions[..9], expected_views);
        assert_eq!(actions[9], "shift 1");
        // Deterministic: the same config gives the same groups.
        assert_eq!(groups, help_groups(&Config::defaults()));
    }

    #[test]
    fn reflects_overrides_removals_and_the_modifier() {
        let config = Config::from_toml(
            r#"
            modifier = "Alt"
            [keybindings]
            "Mod+Shift+Return" = "spawn foot"
            "Mod+b" = "none"
            "Ctrl+Alt+Delete" = "quit"
            [pointer_bindings]
            "Mod+Button3" = "none"
            "#,
        )
        .expect("valid config");
        let groups = help_groups(&config);
        let shown = entries(&groups);
        assert!(shown.contains(&(HelpSection::Launch, "Alt+Shift+Return", "spawn foot")));
        assert!(!shown.iter().any(|(_, _, a)| *a == "spawn-terminal"));
        assert!(!shown.iter().any(|(_, _, a)| *a == "toggle-bar"));
        assert!(shown.contains(&(HelpSection::Session, "Ctrl+Alt+Delete", "quit")));
        assert!(shown.contains(&(HelpSection::Session, "Alt+Shift+slash", "help")));
        assert!(!shown.iter().any(|(_, c, _)| c.contains("Super")));
        let mouse: Vec<_> = shown
            .iter()
            .filter(|(s, ..)| *s == HelpSection::Mouse)
            .collect();
        assert_eq!(mouse, [&(HelpSection::Mouse, "Alt+Button1", "float-move")]);
    }

    #[test]
    fn empty_sections_are_left_out() {
        let mut config = Config::defaults();
        config.pointer_bindings.clear();
        config
            .keybindings
            .retain(|b| classify(&b.action).0 != HelpSection::Screens);
        let sections: Vec<HelpSection> = help_groups(&config).iter().map(|g| g.section).collect();
        assert!(!sections.contains(&HelpSection::Screens));
        assert!(!sections.contains(&HelpSection::Mouse));
        assert_eq!(sections.len(), 5);
    }

    #[test]
    fn keys_close() {
        for key in [PromptKey::Escape, PromptKey::Return, PromptKey::Char('q')] {
            assert_eq!(Help::new().handle_key(key), HelpOutcome::Close, "{key:?}");
        }
        for key in [PromptKey::Char('x'), PromptKey::Tab, PromptKey::Backspace] {
            assert_eq!(
                Help::new().handle_key(key),
                HelpOutcome::Continue,
                "{key:?}"
            );
        }
    }

    #[test]
    fn scrolls_within_the_viewport() {
        let mut help = Help::new();
        // Nothing scrolls until the renderer reports its viewport.
        help.handle_key(PromptKey::Char('j'));
        assert_eq!(help.scroll(), 0);
        help.set_viewport(10, 25);
        for (key, expected) in [
            (PromptKey::Char('j'), 1),
            (PromptKey::Down, 2),
            (PromptKey::Char('k'), 1),
            (PromptKey::Up, 0),
            (PromptKey::Up, 0),
            (PromptKey::PageDown, 9),
            (PromptKey::Char(' '), 18),
            (PromptKey::PageDown, 25),
            (PromptKey::Char('j'), 25),
            (PromptKey::PageUp, 16),
            (PromptKey::Home, 0),
            (PromptKey::End, 25),
        ] {
            assert_eq!(help.handle_key(key), HelpOutcome::Continue, "{key:?}");
            assert_eq!(help.scroll(), expected, "{key:?}");
        }
        // A larger screen clamps the offset.
        help.set_viewport(30, 5);
        assert_eq!((help.scroll(), help.max_scroll()), (5, 5));
        help.set_viewport(40, 0);
        assert_eq!(help.scroll(), 0);
    }
}
