//! Window manage rules, applied when a new toplevel is mapped — the Wayland
//! counterpart of xmonad's `manageHook`.
//!
//! The only rules are the user's `manageZoomHook`: Zoom's main and meeting
//! windows tile (and are pushed out of the master slot), while every other
//! Zoom window — notification popups, the "join audio" dialog, etc. — floats.
//! X11 `className` maps to the Wayland `app_id`.

/// What to do with a freshly mapped window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManageDecision {
    /// Tile the window normally (insert as master).
    Tile,
    /// Float the window at its natural size.
    Float,
    /// Tile the window, but swap it down out of the master slot
    /// (`doSink <+> doF W.swapDown`).
    SinkSwapDown,
}

/// The `app_id` (X11 class) the zoom rules match.
const ZOOM_APP_ID: &str = "zoom";

/// Zoom window titles that should tile; everything else from Zoom floats.
const ZOOM_TILE_TITLES: &[&str] = &[
    // Main window.
    "Zoom - Free Account",
    "Zoom - Licensed Account",
    // Meeting window on creation, and shortly after.
    "Zoom",
    "Zoom Meeting",
];

/// Decides how to manage a new window from its `app_id` and title.
#[must_use]
pub fn manage_decision(app_id: Option<&str>, title: Option<&str>) -> ManageDecision {
    if app_id != Some(ZOOM_APP_ID) {
        return ManageDecision::Tile;
    }
    let title = title.unwrap_or_default();
    if ZOOM_TILE_TITLES.contains(&title) {
        ManageDecision::SinkSwapDown
    } else {
        ManageDecision::Float
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regular_windows_tile() {
        assert_eq!(
            manage_decision(Some("Alacritty"), Some("zsh")),
            ManageDecision::Tile
        );
        assert_eq!(manage_decision(None, None), ManageDecision::Tile);
    }

    #[test]
    fn zoom_main_and_meeting_windows_sink() {
        for title in ZOOM_TILE_TITLES {
            assert_eq!(
                manage_decision(Some("zoom"), Some(title)),
                ManageDecision::SinkSwapDown
            );
        }
    }

    #[test]
    fn other_zoom_windows_float() {
        assert_eq!(
            manage_decision(Some("zoom"), Some("Settings")),
            ManageDecision::Float
        );
        assert_eq!(manage_decision(Some("zoom"), None), ManageDecision::Float);
    }
}
