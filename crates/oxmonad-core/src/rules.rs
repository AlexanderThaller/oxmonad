//! Manage rules: what happens to a window when it is first managed
//! (xmonad's `manageHook`).
//!
//! Rules are evaluated in order and the **first** matching rule decides. A
//! rule matches when every condition it sets matches. Independently of the
//! rules, xmonad's built-in policy floats windows that have a parent
//! (transients/dialogs); a matching rule overrides that.
//!
//! The default rules in `default-config.toml` reproduce the reference
//! config's `manageZoomHook`: Zoom's main and meeting windows tile and are
//! swapped out of the master slot (`doSink <+> doF W.swapDown`), every other
//! Zoom window floats.

use serde::Deserialize;

/// A string pattern: a glob (`*` matches any run of characters, `?` any
/// single character; everything else literally, case-sensitive) or a list
/// of globs of which any may match.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum Pattern {
    /// A single glob.
    One(String),
    /// Matches when any of the globs match.
    AnyOf(Vec<String>),
}

impl Pattern {
    /// Whether `text` matches the pattern.
    #[must_use]
    pub fn matches(&self, text: &str) -> bool {
        match self {
            Self::One(glob) => glob_match(glob, text),
            Self::AnyOf(globs) => globs.iter().any(|glob| glob_match(glob, text)),
        }
    }
}

/// How a matching rule places the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RuleAction {
    /// Float the window, centred on its screen (`doFloat`).
    Float,
    /// Tile the window, even if it would float by default (`doSink`).
    Tile,
    /// Tile the window and swap it one position down after insertion, which
    /// moves it out of the master slot (`doSink <+> doF W.swapDown`).
    TileBelowMaster,
}

/// One manage rule.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    /// Pattern for the window's app id (X11 class under Xwayland).
    #[serde(default)]
    pub app_id: Option<Pattern>,
    /// Pattern for the window's title at manage time.
    #[serde(default)]
    pub title: Option<Pattern>,
    /// Match only windows with (`true`) or without (`false`) a parent.
    #[serde(default)]
    pub has_parent: Option<bool>,
    /// Placement to apply.
    #[serde(default)]
    pub action: Option<RuleAction>,
    /// Send the window to this workspace (`doShift`), creating the
    /// workspace if it does not exist yet.
    #[serde(default)]
    pub workspace: Option<String>,
}

impl Rule {
    /// Whether the rule's conditions all hold for the window.
    #[must_use]
    pub fn matches(&self, app_id: &str, title: &str, has_parent: bool) -> bool {
        self.app_id.as_ref().is_none_or(|p| p.matches(app_id))
            && self.title.as_ref().is_none_or(|p| p.matches(title))
            && self.has_parent.is_none_or(|want| want == has_parent)
    }
}

/// Where a newly managed window goes, as decided by [`decide`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ManageDecision {
    /// Float the window (centred on its screen).
    pub float: bool,
    /// After insertion, swap the window one position down.
    pub swap_down: bool,
    /// Insert into this workspace instead of the current one.
    pub workspace: Option<String>,
}

/// Evaluates xmonad's default transient policy followed by the first
/// matching rule.
#[must_use]
pub fn decide(rules: &[Rule], app_id: &str, title: &str, has_parent: bool) -> ManageDecision {
    let mut decision = ManageDecision {
        float: has_parent,
        ..ManageDecision::default()
    };
    if let Some(rule) = rules
        .iter()
        .find(|rule| rule.matches(app_id, title, has_parent))
    {
        match rule.action {
            Some(RuleAction::Float) => decision.float = true,
            Some(RuleAction::Tile) => decision.float = false,
            Some(RuleAction::TileBelowMaster) => {
                decision.float = false;
                decision.swap_down = true;
            }
            None => {}
        }
        decision.workspace.clone_from(&rule.workspace);
    }
    decision
}

/// Glob matching with `*` and `?` (iterative, with single-star
/// backtracking).
#[must_use]
pub fn glob_match(glob: &str, text: &str) -> bool {
    let pattern: Vec<char> = glob.chars().collect();
    let text: Vec<char> = text.chars().collect();
    let (mut p, mut t) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while t < text.len() {
        match pattern.get(p) {
            Some('*') => {
                star = Some((p, t));
                p += 1;
            }
            Some(&c) if c == '?' || c == text[t] => {
                p += 1;
                t += 1;
            }
            _ => match star {
                Some((sp, st)) => {
                    p = sp + 1;
                    t = st + 1;
                    star = Some((sp, st + 1));
                }
                None => return false,
            },
        }
    }
    pattern[p..].iter().all(|&c| c == '*')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Deserialize)]
    struct Wrapper {
        rules: Vec<Rule>,
    }

    fn zoom_rules() -> Vec<Rule> {
        let toml = r#"
            [[rules]]
            app_id = "zoom"
            title = ["Zoom - Free Account", "Zoom - Licensed Account", "Zoom", "Zoom Meeting"]
            action = "tile-below-master"

            [[rules]]
            app_id = "zoom"
            action = "float"

            [[rules]]
            app_id = "firefox"
            workspace = "web"
        "#;
        toml::from_str::<Wrapper>(toml).expect("rules parse").rules
    }

    #[test]
    fn globs() {
        assert!(glob_match("zoom", "zoom"));
        assert!(!glob_match("zoom", "zoomx"));
        assert!(glob_match("*oom*", "zoom meeting"));
        assert!(glob_match("Zoom?Meeting", "Zoom Meeting"));
        assert!(glob_match("*", ""));
        assert!(glob_match("a*b*c", "axxbyyc"));
        assert!(!glob_match("a*b*c", "axxbyy"));
        assert!(glob_match("*.pdf", "report.pdf"));
    }

    #[test]
    fn zoom_main_and_meeting_windows_tile_below_master() {
        let rules = zoom_rules();
        for title in ["Zoom - Free Account", "Zoom", "Zoom Meeting"] {
            assert_eq!(
                decide(&rules, "zoom", title, false),
                ManageDecision {
                    float: false,
                    swap_down: true,
                    workspace: None
                }
            );
        }
    }

    #[test]
    fn other_zoom_windows_float() {
        let rules = zoom_rules();
        let decision = decide(&rules, "zoom", "Chat", false);
        assert!(decision.float);
        assert!(!decision.swap_down);
    }

    #[test]
    fn transients_float_unless_a_rule_says_otherwise() {
        let rules = zoom_rules();
        assert!(decide(&rules, "gimp", "Open File", true).float);
        assert!(!decide(&rules, "gimp", "GIMP", false).float);
        // A Zoom meeting dialog with a parent still tiles: rules win.
        assert!(!decide(&rules, "zoom", "Zoom Meeting", true).float);
    }

    #[test]
    fn workspace_rule() {
        let rules = zoom_rules();
        let decision = decide(&rules, "firefox", "Mozilla Firefox", false);
        assert_eq!(decision.workspace.as_deref(), Some("web"));
        assert!(!decision.float);
    }

    #[test]
    fn has_parent_condition() {
        let rule = Rule {
            app_id: None,
            title: None,
            has_parent: Some(false),
            action: Some(RuleAction::Float),
            workspace: None,
        };
        assert!(rule.matches("x", "y", false));
        assert!(!rule.matches("x", "y", true));
    }
}
