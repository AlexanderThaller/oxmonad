use super::*;

#[test]
fn manage_inserts_above_focus_and_focuses() {
    let mut wm = wm();
    manage_all(&mut wm, [1, 2]);
    wm.focus_window(w(1));
    manage_all(&mut wm, [3]);
    assert_eq!(windows(&wm), [w(2), w(3), w(1)]);
    assert_eq!(wm.stack_set().peek(), Some(w(3)));
    assert!(wm.is_managed(w(3)));
}

#[test]
fn manage_twice_updates_info_only() {
    let mut wm = wm();
    manage_all(&mut wm, [1]);
    wm.manage(w(1), info("app", "renamed"));
    assert_eq!(windows(&wm), [w(1)]);
    assert_eq!(
        wm.window_info(w(1)).map(|i| i.title.as_str()),
        Some("renamed")
    );
}

#[test]
fn transients_float_centered() {
    let mut wm = wm();
    manage_all(&mut wm, [1]);
    let dialog = WindowInfo {
        app_id: "gimp".into(),
        title: "Open".into(),
        has_parent: true,
        preferred_size: Some(Size::new(400, 300)),
    };
    wm.manage(w(2), dialog);
    assert!(wm.stack_set().is_floating(w(2)));
    let p = wm.arrange().placement(w(2)).cloned().expect("placed");
    assert_eq!(p.layer, Layer::Floating);
    // Content 400x300 plus a 1px border, centred on the full screen.
    assert_eq!(p.rect, Rect::new(760, 390, 400, 300));
    // Without a preferred size it takes the middle quarter of the screen.
    let mut dialog = info("gimp", "Other");
    dialog.has_parent = true;
    wm.manage(w(3), dialog);
    assert_eq!(
        wm.arrange().placement(w(3)).map(|p| p.rect.outset(1)),
        Some(Rect::new(480, 270, 960, 540))
    );
}

#[test]
fn zoom_rules_from_default_config() {
    let mut wm = wm();
    manage_all(&mut wm, [1]);
    wm.manage(w(2), info("zoom", "Zoom Meeting"));
    // Tiled and pushed out of the master slot.
    assert!(!wm.stack_set().is_floating(w(2)));
    assert_eq!(windows(&wm), [w(1), w(2)]);
    wm.manage(w(3), info("zoom", "Chat"));
    assert!(wm.stack_set().is_floating(w(3)));
    // Alone on a workspace a tiled Zoom window simply becomes master.
    apply(&mut wm, "view 2");
    wm.manage(w(4), info("zoom", "Zoom - Free Account"));
    assert_eq!(windows(&wm), [w(4)]);
}

#[test]
fn workspace_rule_sends_window_elsewhere() {
    let mut config = Config::defaults();
    config.rules.push(crate::rules::Rule {
        app_id: Some(crate::rules::Pattern::One("firefox".into())),
        title: None,
        has_parent: None,
        action: None,
        workspace: Some("web".into()),
    });
    let mut wm = Wm::new(config);
    wm.add_output(O0, "DP-1", SCREEN);
    manage_all(&mut wm, [1]);
    wm.manage(w(2), info("firefox", "Mozilla Firefox"));
    assert_eq!(windows(&wm), [w(1)]);
    assert_eq!(wm.stack_set().peek(), Some(w(1)));
    assert_eq!(
        wm.stack_set().workspace("web").map(Workspace::windows),
        Some(vec![w(2)])
    );
    assert_eq!(wm.arrange().hidden, [w(2)]);
}

#[test]
fn unmanage_forgets_everything() {
    let mut wm = wm();
    manage_all(&mut wm, [1, 2]);
    apply(&mut wm, "copy 3");
    wm.request_fullscreen(w(2), true);
    wm.unmanage(w(2));
    assert!(!wm.is_managed(w(2)));
    assert!(!wm.is_fullscreen(w(2)));
    assert!(!wm.stack_set().member(w(2)));
    assert_eq!(wm.stack_set().peek(), Some(w(1)));
    wm.unmanage(w(42));
}

#[test]
fn update_info_ignores_unknown_windows() {
    let mut wm = wm();
    wm.update_info(w(1), info("a", "b"));
    assert!(!wm.is_managed(w(1)));
    wm.focus_window(w(1));
    wm.float_at(w(1), Rect::new(0, 0, 10, 10));
    assert!(!wm.stack_set().is_floating(w(1)));
}

#[test]
fn reconfigure_keeps_state_and_adds_workspaces() {
    let mut wm = wm();
    manage_all(&mut wm, [1]);
    apply(&mut wm, "toggle-bar");
    let mut config = Config::defaults();
    config.workspaces.push("extra".into());
    config.terminal = "foot".into();
    wm.reconfigure(config);
    // The workspace's bar toggle survives a reload.
    assert!(!wm.bar_visible());
    assert!(wm.stack_set().has_tag("extra"));
    assert_eq!(windows(&wm), [w(1)]);
    assert_eq!(
        apply(&mut wm, "spawn-terminal"),
        [Effect::Spawn("foot".into())]
    );
}
