//! `[border] smart` (xmonad-contrib's `smartBorders`).

use super::*;
use crate::config::SmartBorders;

/// A one-screen `Wm` with `smart` borders.
fn wm_with(smart: SmartBorders) -> Wm {
    let mut config = Config::defaults();
    config.border.smart = smart;
    let mut wm = Wm::new(config);
    wm.add_output(O0, "DP-1", SCREEN);
    wm
}

fn has_border(arrangement: &Arrangement, id: u64) -> bool {
    arrangement
        .placement(w(id))
        .and_then(|p| p.border)
        .is_some()
}

#[test]
fn single_screen_is_the_default() {
    assert_eq!(Config::defaults().border.smart, SmartBorders::SingleScreen);
}

#[test]
fn a_lone_tiled_window_has_no_border() {
    let mut wm = wm();
    manage_all(&mut wm, [1]);
    let a = wm.arrange();
    assert!(!has_border(&a, 1));
    assert_eq!(rect_of(&a, 1), Some(AREA));

    manage_all(&mut wm, [2]);
    let a = wm.arrange();
    assert!(has_border(&a, 1) && has_border(&a, 2));
}

#[test]
fn a_layout_showing_one_window_counts_as_lone() {
    // Tabs places only the focused window, like xmonad's Full/tabbed.
    let mut wm = wm();
    manage_all(&mut wm, [1, 2, 3]);
    for _ in 0..4 {
        apply(&mut wm, "next-layout");
    }
    assert_eq!(wm.bar_model().layout, "Tabs");
    let a = wm.arrange();
    assert_eq!(a.windows.len(), 1);
    assert!(!has_border(&a, 3));
}

#[test]
fn off_keeps_every_border() {
    let mut wm = wm_with(SmartBorders::Off);
    manage_all(&mut wm, [1]);
    let a = wm.arrange();
    assert!(has_border(&a, 1));
    assert_eq!(rect_of(&a, 1), Some(AREA.inset(1)));
}

#[test]
fn single_screen_keeps_borders_with_several_screens() {
    let mut wm = wm2();
    manage_all(&mut wm, [1]);
    assert!(has_border(&wm.arrange(), 1));

    let mut config = Config::defaults();
    config.border.smart = SmartBorders::PerScreen;
    wm.reconfigure(config);
    assert!(!has_border(&wm.arrange(), 1));
}

#[test]
fn a_split_screen_counts_as_several_screens() {
    let mut wm = wm();
    manage_all(&mut wm, [1]);
    apply(&mut wm, "split-screen 2");
    assert!(has_border(&wm.arrange(), 1));

    let mut wm = wm_with(SmartBorders::PerScreen);
    manage_all(&mut wm, [1]);
    apply(&mut wm, "split-screen 2");
    assert!(!has_border(&wm.arrange(), 1));
}

#[test]
fn a_float_covering_its_screen_has_no_border() {
    for smart in [SmartBorders::SingleScreen, SmartBorders::PerScreen] {
        let mut wm = wm_with(smart);
        manage_all(&mut wm, [1, 2, 3]);
        wm.float_at(w(1), SCREEN);
        wm.float_at(w(2), Rect::new(100, 100, 200, 200));
        let a = wm.arrange();
        assert!(!has_border(&a, 1), "{smart:?}");
        assert_eq!(rect_of(&a, 1), Some(SCREEN));
        assert!(has_border(&a, 2), "{smart:?}");
    }

    let mut wm = wm_with(SmartBorders::Off);
    manage_all(&mut wm, [1, 2]);
    wm.float_at(w(1), SCREEN);
    assert!(has_border(&wm.arrange(), 1));
}
