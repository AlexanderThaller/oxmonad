//! `Wm` tests. Helpers live here; the tests are split by topic.

mod actions;
mod arrange;
mod manage;
mod multimon;
mod prompt;
mod restore;
mod scratchpad;
mod split;

use super::*;
use crate::{
    action::Action,
    config::Config,
    geometry::ScreenId,
    stack_set::Workspace,
};

const S0: ScreenId = ScreenId(0);
const S1: ScreenId = ScreenId(1);
const O0: OutputId = OutputId(0);
const O1: OutputId = OutputId(1);

/// The full screen of the default single-screen setup.
const SCREEN: Rect = Rect::new(0, 0, 1920, 1080);
/// The tiling area below the default 24px bar.
const AREA: Rect = Rect::new(0, 24, 1920, 1056);

fn w(n: u64) -> WindowId {
    WindowId(n)
}

fn info(app_id: &str, title: &str) -> WindowInfo {
    WindowInfo {
        app_id: app_id.into(),
        title: title.into(),
        has_parent: false,
        preferred_size: None,
    }
}

/// A `Wm` with the default config and one 1920x1080 screen.
fn wm() -> Wm {
    let mut wm = Wm::new(Config::defaults());
    wm.add_output(O0, "DP-1", SCREEN);
    wm
}

/// A `Wm` with two side-by-side 1920x1080 screens.
fn wm2() -> Wm {
    let mut wm = wm();
    wm.add_output(O1, "DP-2", Rect::new(1920, 0, 1920, 1080));
    wm
}

/// Manages windows `ids` in order with a generic app id.
fn manage_all(wm: &mut Wm, ids: impl IntoIterator<Item = u64>) {
    for id in ids {
        wm.manage(w(id), info("app", &format!("window {id}")));
    }
}

fn apply(wm: &mut Wm, command: &str) -> Vec<Effect> {
    wm.apply(command.parse::<Action>().expect("valid action"))
}

fn windows(wm: &Wm) -> Vec<WindowId> {
    wm.stack_set().current_workspace().windows()
}

fn rect_of(arrangement: &Arrangement, id: u64) -> Option<Rect> {
    arrangement.placement(w(id)).map(|p| p.rect)
}
