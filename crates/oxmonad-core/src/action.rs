//! User intents ([`Action`]) and the side effects core asks the binary to
//! perform ([`Effect`]).
//!
//! Actions have a kebab-case command syntax used by the config's keybinding
//! table, e.g. `spawn alacritty`, `view 3`, `prompt move` or
//! `toggle-scratchpad term`; [`Action`] implements [`FromStr`] and
//! [`fmt::Display`] for it (the two round-trip).

use std::{
    fmt,
    str::FromStr,
};

use crate::{
    geometry::WindowId,
    prompt::PromptKind,
};

/// A direction in an ordered cycle (screens in physical order).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Direction {
    /// The next element, wrapping.
    Next,
    /// The previous element, wrapping.
    Prev,
}

impl Direction {
    fn as_str(self) -> &'static str {
        match self {
            Self::Next => "next",
            Self::Prev => "prev",
        }
    }
}

/// Everything a binding can ask the window manager to do.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Action {
    /// Run a shell command (`spawn <command…>`).
    Spawn(String),
    /// Run the configured terminal (`spawn-terminal`).
    SpawnTerminal,
    /// Focus the next window in stack order, wrapping (`focus-next`,
    /// xmonad `focusDown`).
    FocusNext,
    /// Focus the previous window, wrapping (`focus-prev`, `focusUp`).
    FocusPrev,
    /// Focus the master window (`focus-master`).
    FocusMaster,
    /// Swap the focused window with the next one (`swap-next`, `swapDown`).
    SwapNext,
    /// Swap the focused window with the previous one (`swap-prev`,
    /// `swapUp`).
    SwapPrev,
    /// Swap the focused window with the master (`swap-master`).
    SwapMaster,
    /// Push the focused floating window back into the tiling (`sink`).
    Sink,
    /// Close the focused window (`close`, xmonad `kill`).
    Close,
    /// Rotate to the next layout in the ring (`next-layout`).
    NextLayout,
    /// Rebuild the current workspace's layout ring from the defaults
    /// (`reset-layout`).
    ResetLayout,
    /// Shrink the master area (`shrink`).
    Shrink,
    /// Expand the master area (`expand`).
    Expand,
    /// `ResizableTall`'s `MirrorShrink` (`mirror-shrink`).
    MirrorShrink,
    /// `ResizableTall`'s `MirrorExpand` (`mirror-expand`).
    MirrorExpand,
    /// Change the number of master windows (`inc-master <n>`).
    IncMaster(i32),
    /// Show a workspace on the current screen, swapping with another screen
    /// if it is visible there (`view <tag>`, xmonad `greedyView`).
    View(String),
    /// Move the focused window to a workspace (`shift <tag>`).
    Shift(String),
    /// Copy the focused window to a workspace (`copy <tag>`).
    Copy(String),
    /// View the most recently hidden workspace (`toggle-ws`,
    /// `toggleWS' ["NSP"]`).
    ToggleWs,
    /// Open a workspace prompt (`prompt view|move|copy|rename`).
    Prompt(PromptKind),
    /// Remove the current workspace, merging its windows into the most
    /// recently hidden one (`remove-workspace`).
    RemoveWorkspace,
    /// Focus the next/previous screen in physical order
    /// (`focus-screen next|prev`, `onNextNeighbour def W.view`).
    FocusScreen(Direction),
    /// Send the focused window to the next/previous screen's workspace,
    /// focus stays (`shift-screen next|prev`).
    ShiftScreen(Direction),
    /// Send the focused window to the next/previous screen and follow it
    /// (`shift-screen-follow next|prev`).
    ShiftScreenFollow(Direction),
    /// Swap the current screen's workspace with the next screen's
    /// (`swap-next-screen`, `swapNextScreen`).
    SwapNextScreen,
    /// Split the current screen into `n` (2 to [`MAX_SPLIT`]) virtual
    /// screens side by side (`split-screen <n>`,
    /// `layoutSplitScreen n (TwoPane 0.5 0.5)`).
    SplitScreen(usize),
    /// Undo every screen split: one screen per output again (`rescreen`,
    /// `XMonad.Actions.PhysicalScreens.rescreen`).
    Rescreen,
    /// Show or hide the bar (`toggle-bar`, `ToggleStruts`).
    ToggleBar,
    /// Re-arrange everything (`refresh`).
    Refresh,
    /// Quit oxmonad (`quit`).
    Quit,
    /// Reload the configuration (`reload`).
    Reload,
    /// Replace oxmonad with a fresh copy of its binary, keeping the
    /// windows and their placement (`restart`, xmonad's `Mod+q`).
    Restart,
    /// Toggle fullscreen for the focused window (`toggle-fullscreen`).
    ToggleFullscreen,
    /// Show/hide/spawn a named scratchpad (`toggle-scratchpad <name>`).
    ToggleScratchpad(String),
    /// Float the focused window and start an interactive pointer move
    /// (`float-move`, xmonad's `mouseMoveWindow`).
    FloatMove,
    /// Float the focused window and start an interactive pointer resize
    /// (`float-resize`, `mouseResizeWindow`).
    FloatResize,
}

/// Side effects core wants the outside world to perform. Core never does
/// I/O itself; the binary executes these.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Effect {
    /// Run a shell command.
    Spawn(String),
    /// Ask a window to close.
    Close(WindowId),
    /// Exit the window manager.
    Quit,
    /// Re-read the configuration and pass it to `Wm::reconfigure`.
    Reload,
    /// Restart in place: save `Wm::snapshot`, re-execute the binary and
    /// restore the snapshot there.
    Restart,
    /// Start an interactive pointer move of a (now floating) window; report
    /// the new geometry with `Wm::set_float_rect`.
    StartPointerMove(WindowId),
    /// Start an interactive pointer resize of a (now floating) window.
    StartPointerResize(WindowId),
}

/// Error returned when an action command string cannot be parsed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ParseActionError {
    /// The command is empty.
    #[error("empty action")]
    Empty,
    /// The command name is not known.
    #[error("unknown action `{0}`")]
    Unknown(String),
    /// The command needs an argument that is missing.
    #[error("action `{0}` needs an argument")]
    MissingArgument(&'static str),
    /// The command takes no argument but one was given.
    #[error("action `{0}` takes no argument")]
    UnexpectedArgument(&'static str),
    /// The argument is not valid for the command.
    #[error("invalid argument `{argument}` for action `{action}`")]
    InvalidArgument {
        /// The command name.
        action: &'static str,
        /// The offending argument.
        argument: String,
    },
}

/// The most screens `split-screen` splits a screen into.
pub const MAX_SPLIT: usize = 16;

/// Commands without an argument, with their action.
const PLAIN: &[(&str, Action)] = &[
    ("spawn-terminal", Action::SpawnTerminal),
    ("focus-next", Action::FocusNext),
    ("focus-prev", Action::FocusPrev),
    ("focus-master", Action::FocusMaster),
    ("swap-next", Action::SwapNext),
    ("swap-prev", Action::SwapPrev),
    ("swap-master", Action::SwapMaster),
    ("sink", Action::Sink),
    ("close", Action::Close),
    ("next-layout", Action::NextLayout),
    ("reset-layout", Action::ResetLayout),
    ("shrink", Action::Shrink),
    ("expand", Action::Expand),
    ("mirror-shrink", Action::MirrorShrink),
    ("mirror-expand", Action::MirrorExpand),
    ("toggle-ws", Action::ToggleWs),
    ("remove-workspace", Action::RemoveWorkspace),
    ("swap-next-screen", Action::SwapNextScreen),
    ("rescreen", Action::Rescreen),
    ("toggle-bar", Action::ToggleBar),
    ("refresh", Action::Refresh),
    ("quit", Action::Quit),
    ("reload", Action::Reload),
    ("restart", Action::Restart),
    ("toggle-fullscreen", Action::ToggleFullscreen),
    ("float-move", Action::FloatMove),
    ("float-resize", Action::FloatResize),
];

impl FromStr for Action {
    type Err = ParseActionError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        let (name, arg) = match s.split_once(char::is_whitespace) {
            Some((name, arg)) => (name, Some(arg.trim()).filter(|arg| !arg.is_empty())),
            None => (s, None),
        };
        if name.is_empty() {
            return Err(ParseActionError::Empty);
        }
        if let Some((static_name, action)) = PLAIN.iter().find(|(n, _)| *n == name) {
            return match arg {
                None => Ok(action.clone()),
                Some(_) => Err(ParseActionError::UnexpectedArgument(static_name)),
            };
        }
        let Some(static_name) = WITH_ARGUMENT.iter().copied().find(|n| *n == name) else {
            return Err(ParseActionError::Unknown(name.to_owned()));
        };
        let arg = arg.ok_or(ParseActionError::MissingArgument(static_name))?;
        match static_name {
            "spawn" => Ok(Self::Spawn(arg.to_owned())),
            "view" => Ok(Self::View(arg.to_owned())),
            "shift" => Ok(Self::Shift(arg.to_owned())),
            "copy" => Ok(Self::Copy(arg.to_owned())),
            "toggle-scratchpad" => Ok(Self::ToggleScratchpad(arg.to_owned())),
            "inc-master" => arg
                .parse()
                .map(Self::IncMaster)
                .map_err(|_| invalid(static_name, arg)),
            "split-screen" => arg
                .parse()
                .ok()
                .filter(|n| (2..=MAX_SPLIT).contains(n))
                .map(Self::SplitScreen)
                .ok_or_else(|| invalid(static_name, arg)),
            "prompt" => PromptKind::from_command(arg)
                .map(Self::Prompt)
                .ok_or_else(|| invalid(static_name, arg)),
            "focus-screen" => direction(static_name, arg).map(Self::FocusScreen),
            "shift-screen" => direction(static_name, arg).map(Self::ShiftScreen),
            _ => direction(static_name, arg).map(Self::ShiftScreenFollow),
        }
    }
}

/// Commands that take an argument.
const WITH_ARGUMENT: &[&str] = &[
    "spawn",
    "view",
    "shift",
    "copy",
    "toggle-scratchpad",
    "inc-master",
    "split-screen",
    "prompt",
    "focus-screen",
    "shift-screen",
    "shift-screen-follow",
];

fn invalid(action: &'static str, argument: &str) -> ParseActionError {
    ParseActionError::InvalidArgument {
        action,
        argument: argument.to_owned(),
    }
}

fn direction(action: &'static str, arg: &str) -> Result<Direction, ParseActionError> {
    match arg {
        "next" => Ok(Direction::Next),
        "prev" => Ok(Direction::Prev),
        _ => Err(invalid(action, arg)),
    }
}

impl fmt::Display for Action {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some((name, _)) = PLAIN.iter().find(|(_, action)| action == self) {
            return f.write_str(name);
        }
        match self {
            Self::Spawn(cmd) => write!(f, "spawn {cmd}"),
            Self::View(tag) => write!(f, "view {tag}"),
            Self::Shift(tag) => write!(f, "shift {tag}"),
            Self::Copy(tag) => write!(f, "copy {tag}"),
            Self::ToggleScratchpad(name) => write!(f, "toggle-scratchpad {name}"),
            Self::IncMaster(n) => write!(f, "inc-master {n}"),
            Self::SplitScreen(n) => write!(f, "split-screen {n}"),
            Self::Prompt(kind) => write!(f, "prompt {}", kind.command()),
            Self::FocusScreen(d) => write!(f, "focus-screen {}", d.as_str()),
            Self::ShiftScreen(d) => write!(f, "shift-screen {}", d.as_str()),
            Self::ShiftScreenFollow(d) => write!(f, "shift-screen-follow {}", d.as_str()),
            Self::SpawnTerminal
            | Self::FocusNext
            | Self::FocusPrev
            | Self::FocusMaster
            | Self::SwapNext
            | Self::SwapPrev
            | Self::SwapMaster
            | Self::Sink
            | Self::Close
            | Self::NextLayout
            | Self::ResetLayout
            | Self::Shrink
            | Self::Expand
            | Self::MirrorShrink
            | Self::MirrorExpand
            | Self::ToggleWs
            | Self::RemoveWorkspace
            | Self::SwapNextScreen
            | Self::Rescreen
            | Self::ToggleBar
            | Self::Refresh
            | Self::Quit
            | Self::Reload
            | Self::Restart
            | Self::ToggleFullscreen
            | Self::FloatMove
            | Self::FloatResize => unreachable!("plain actions are listed in PLAIN"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> Action {
        s.parse().expect("action parses")
    }

    #[test]
    fn parses_commands_with_arguments() {
        assert_eq!(
            parse("spawn tofi-run | xargs --no-run-if-empty setsid --fork"),
            Action::Spawn("tofi-run | xargs --no-run-if-empty setsid --fork".into())
        );
        assert_eq!(parse("view 3"), Action::View("3".into()));
        assert_eq!(parse("  shift   web "), Action::Shift("web".into()));
        assert_eq!(parse("copy 2"), Action::Copy("2".into()));
        assert_eq!(parse("inc-master -1"), Action::IncMaster(-1));
        assert_eq!(parse("split-screen 2"), Action::SplitScreen(2));
        assert_eq!(parse("rescreen"), Action::Rescreen);
        assert_eq!(
            parse("prompt move"),
            Action::Prompt(PromptKind::MoveToWorkspace)
        );
        assert_eq!(
            parse("focus-screen prev"),
            Action::FocusScreen(Direction::Prev)
        );
        assert_eq!(
            parse("toggle-scratchpad term"),
            Action::ToggleScratchpad("term".into())
        );
    }

    #[test]
    fn rejects_bad_commands() {
        assert_eq!("".parse::<Action>(), Err(ParseActionError::Empty));
        assert_eq!(
            "frobnicate".parse::<Action>(),
            Err(ParseActionError::Unknown("frobnicate".into()))
        );
        assert_eq!(
            "view".parse::<Action>(),
            Err(ParseActionError::MissingArgument("view"))
        );
        assert_eq!(
            "quit now".parse::<Action>(),
            Err(ParseActionError::UnexpectedArgument("quit"))
        );
        assert!(matches!(
            "inc-master many".parse::<Action>(),
            Err(ParseActionError::InvalidArgument { .. })
        ));
        for bad in ["split-screen 1", "split-screen 17", "split-screen two"] {
            assert!(
                matches!(
                    bad.parse::<Action>(),
                    Err(ParseActionError::InvalidArgument { .. })
                ),
                "{bad}"
            );
        }
        assert!(matches!(
            "focus-screen up".parse::<Action>(),
            Err(ParseActionError::InvalidArgument { .. })
        ));
    }

    #[test]
    fn display_round_trips() {
        let mut all: Vec<Action> = PLAIN.iter().map(|(_, a)| a.clone()).collect();
        all.extend([
            Action::Spawn("emacs".into()),
            Action::View("1".into()),
            Action::Shift("9".into()),
            Action::Copy("x".into()),
            Action::ToggleScratchpad("term".into()),
            Action::IncMaster(1),
            Action::SplitScreen(3),
            Action::FocusScreen(Direction::Next),
            Action::ShiftScreen(Direction::Prev),
            Action::ShiftScreenFollow(Direction::Next),
        ]);
        all.extend(PromptKind::ALL.iter().copied().map(Action::Prompt));
        for action in all {
            assert_eq!(parse(&action.to_string()), action, "{action}");
        }
    }
}
