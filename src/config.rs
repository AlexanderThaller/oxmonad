//! Compile-time configuration for oxmonad.
//!
//! Keybindings and a few program choices are hardcoded here for now. This is
//! intended to be replaced by a proper runtime/user configuration mechanism
//! later, but keeping it centralised makes the eventual migration easy.

use smithay::{
    backend::renderer::Color32F,
    input::keyboard::ModifiersState,
};

use crate::layout::Workspace;

/// The primary modifier used by every oxmonad keybinding — xmonad's "mod key".
///
/// Defaults to [`Modifier::Super`] (the "logo"/Windows key). Change this single
/// constant to rebind every shortcut to a different modifier.
pub const META: Modifier = Modifier::Super;

/// Terminal emulator launched by the spawn-terminal keybinding.
pub const TERMINAL: &str = "alacritty";

/// Application launcher, run through `sh -c`.
///
/// `tofi-run` only *prints* the chosen command (dmenu_run-style, picking a
/// binary from `$PATH`), so we pipe it through `xargs` to actually execute the
/// selection, detached via `setsid` so it outlives the pipeline. This mirrors
/// the Sway `tofi-run | xargs swaymsg exec --` recipe. Switch to
/// `tofi-drun --drun-launch=true` to pick and launch `.desktop` apps instead.
///
/// tofi is a Wayland layer-shell client, so it relies on oxmonad's
/// `wlr-layer-shell` support.
pub const LAUNCHER: &str = "tofi-run | xargs --no-run-if-empty setsid --fork";

/// Editor launched by the spawn-editor keybinding (xmonad config's `mod+x`).
pub const EDITOR: &str = "emacs";

/// Screen locker launched by the lock keybinding, run through `sh -c`.
///
/// The xmonad config spawns `slock`, which is X11-only; `swaylock` is the
/// Wayland equivalent (it uses the `ext-session-lock` protocol).
pub const LOCKER: &str = "swaylock";

/// Command run when a media play/pause key is pressed, through `sh -c`.
pub const MEDIA_PLAY_PAUSE: &str = "playerctl play-pause";

/// Command run when the media next-track key is pressed, through `sh -c`.
pub const MEDIA_NEXT: &str = "playerctl next";

/// Command run when the media previous-track key is pressed, through `sh -c`.
pub const MEDIA_PREV: &str = "playerctl previous";

/// Name of the primary output — the monitor that carries the built-in bar.
///
/// Outputs are named after their connector, e.g. `"DP-1"`, `"HDMI-A-1"` or
/// `"eDP-1"` (run oxmonad and check the logs to see the detected names). When
/// `None`, or when no connected output matches the name, the first detected
/// output becomes primary. Every other monitor tiles windows without a bar.
pub const PRIMARY_OUTPUT: Option<&str> = None;

/// Names of the workspaces (virtual desktops) created at startup, in order.
///
/// Defaults to the xmonad-style numeric tags `"1"`..`"9"`; replace any entry
/// with a custom label (e.g. `"web"`, `"code"`, `"chat"`) for named
/// workspaces. They are addressed by position: `META + 1` always views the
/// first workspace regardless of its name. There must be at least one, and at
/// least as many as the number of connected monitors for each to get a
/// distinct workspace.
pub const WORKSPACES: &[&str] = &["1", "2", "3", "4", "5", "6", "7", "8", "9"];

/// Builds the initial [`Workspace`] pool from [`WORKSPACES`].
///
/// Falls back to a single `"1"` workspace if [`WORKSPACES`] is empty, so the
/// compositor always has somewhere to place windows.
#[must_use]
pub fn workspaces() -> Vec<Workspace> {
    if WORKSPACES.is_empty() {
        return vec![Workspace::new("1".to_string())];
    }
    WORKSPACES
        .iter()
        .map(|name| Workspace::new((*name).to_string()))
        .collect()
}

/// Height of the built-in status bar, in logical pixels.
pub const BAR_HEIGHT: i32 = 24;

/// Background color of the built-in status bar.
pub const BAR_BACKGROUND_COLOR: Color32F =
    Color32F::new(0.062_745_1, 0.066_666_7, 0.078_431_375, 1.0);

/// Text color of the built-in status bar.
pub const BAR_TEXT_COLOR: Color32F = Color32F::new(0.827_451, 0.843_137_26, 0.878_431_4, 1.0);

/// Text color of the workspace shown on a monitor, in its bar — yellow
/// (`#F8E63A`), matching the user's xmonad `ppCurrent`.
pub const BAR_WORKSPACE_ACTIVE_COLOR: Color32F =
    Color32F::new(0.972_549, 0.901_960_8, 0.227_451, 1.0);

/// Text color of the focused window title in the bar — green (`#86BD47`),
/// matching the user's xmonad `ppTitle`.
pub const BAR_TITLE_COLOR: Color32F = Color32F::new(0.525_490_2, 0.741_176_5, 0.278_431_37, 1.0);

/// Background color of the workspace prompt — dark grey (`#161616`),
/// matching the xmonad `myXPConfig` `bgColor`.
pub const PROMPT_BG_COLOR: Color32F = Color32F::new(0.086_274_51, 0.086_274_51, 0.086_274_51, 1.0);

/// Text color of the workspace prompt — white (`#F2F2F2`, `fgColor`).
pub const PROMPT_FG_COLOR: Color32F = Color32F::new(0.949_019_6, 0.949_019_6, 0.949_019_6, 1.0);

/// Border color of the workspace prompt — white (`#F2F2F2`, `borderColor`).
pub const PROMPT_BORDER_COLOR: Color32F = Color32F::new(0.949_019_6, 0.949_019_6, 0.949_019_6, 1.0);

/// Background color of the highlighted completion — blue (`#1565C1`,
/// `bgHLight`).
pub const PROMPT_HL_BG_COLOR: Color32F =
    Color32F::new(0.082_352_94, 0.396_078_43, 0.756_862_75, 1.0);

/// Text color of the highlighted completion — white (`#F2F2F2`, `fgHLight`).
pub const PROMPT_HL_FG_COLOR: Color32F = Color32F::new(0.949_019_6, 0.949_019_6, 0.949_019_6, 1.0);

/// Width of the window border, in logical pixels.
pub const BORDER_WIDTH: i32 = 1;

/// Border color of unfocused windows — light grey (`#969694`).
pub const BORDER_COLOR_NORMAL: Color32F = Color32F::new(0.588_235_3, 0.588_235_3, 0.580_392_2, 1.0);

/// Border color of the focused window — red (`#C62828`).
pub const BORDER_COLOR_FOCUSED: Color32F =
    Color32F::new(0.776_470_6, 0.156_862_7, 0.156_862_7, 1.0);

/// A logical modifier key that a keybinding can require.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[expect(
    dead_code,
    reason = "`Alt`/`Control` are selectable values for `META`; only the currently chosen variant \
              is constructed."
)]
pub enum Modifier {
    /// The "Super"/"logo"/Windows key.
    Super,
    /// The Alt key.
    Alt,
    /// The Control key.
    Control,
}

impl Modifier {
    /// Returns whether this modifier is currently held in `modifiers`.
    #[must_use]
    pub fn is_active(self, modifiers: &ModifiersState) -> bool {
        match self {
            Self::Super => modifiers.logo,
            Self::Alt => modifiers.alt,
            Self::Control => modifiers.ctrl,
        }
    }
}
