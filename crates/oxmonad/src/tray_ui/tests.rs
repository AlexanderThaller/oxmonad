use oxmonad_core::{
    Rect,
    prompt::PromptKey,
};
use oxmonad_ui::{
    Fonts,
    Icon,
    MenuEntryKind,
    MenuStyle,
    menu_size,
};

use super::*;
use crate::tray::{
    ItemStatus,
    MenuKind,
    MenuNode,
    Toggle,
    TrayIcon,
    TrayItem,
};

fn node(id: i32, label: &str, children: Vec<MenuNode>) -> MenuNode {
    MenuNode {
        id,
        label: label.to_owned(),
        kind: MenuKind::Normal,
        toggle: Toggle::None,
        enabled: true,
        visible: true,
        children,
    }
}

fn separator(id: i32) -> MenuNode {
    MenuNode {
        kind: MenuKind::Separator,
        ..node(id, "", vec![])
    }
}

fn icon() -> TrayIcon {
    TrayIcon {
        width: 1,
        height: 1,
        bgra: vec![1, 2, 3, 255],
    }
}

fn item(id: &str, status: ItemStatus, icon: Option<TrayIcon>) -> TrayItem {
    TrayItem {
        id: id.to_owned(),
        title: id.to_uppercase(),
        status,
        icon,
        item_is_menu: false,
        menu: None,
    }
}

fn bar_item(menu: Option<MenuNode>, item_is_menu: bool) -> BarItem {
    BarItem {
        item_is_menu,
        menu,
        ..BarItem::new(item("a", ItemStatus::Active, Some(icon())))
    }
}

#[test]
fn items_convert_their_icon_once() {
    let items = bar_items(vec![
        item("a", ItemStatus::Active, Some(icon())),
        item("b", ItemStatus::Active, None),
    ]);
    assert_eq!(
        items[0].icon,
        Icon::from_premultiplied_bgra(1, 1, vec![1, 2, 3, 255])
    );
    assert_eq!(items[1].icon, None);
    assert_eq!(items[1].title, "B");
}

#[test]
fn segments_skip_passive_and_iconless_items_in_order() {
    let items = bar_items(vec![
        item("a", ItemStatus::Active, Some(icon())),
        item("passive", ItemStatus::Passive, Some(icon())),
        item("no-icon", ItemStatus::Active, None),
        item("c", ItemStatus::NeedsAttention, Some(icon())),
    ]);
    let actions: Vec<Option<String>> = segments(&items)
        .into_iter()
        .map(|segment| {
            assert!(segment.icon.is_some());
            segment.action
        })
        .collect();
    assert_eq!(actions, vec![Some("a".to_owned()), Some("c".to_owned())]);
}

#[test]
fn buttons_map_to_tray_calls() {
    let menu = Some(node(0, "", vec![node(1, "Quit", vec![])]));
    let plain = bar_item(None, false);
    assert_eq!(icon_action(&plain, Button::Left), IconAction::Activate);
    assert_eq!(
        icon_action(&plain, Button::Middle),
        IconAction::SecondaryActivate
    );
    assert_eq!(icon_action(&plain, Button::Right), IconAction::ContextMenu);

    let with_menu = bar_item(menu.clone(), false);
    assert_eq!(icon_action(&with_menu, Button::Left), IconAction::Activate);
    assert_eq!(icon_action(&with_menu, Button::Right), IconAction::OpenMenu);

    let is_menu = bar_item(menu, true);
    assert_eq!(icon_action(&is_menu, Button::Left), IconAction::OpenMenu);
    assert_eq!(icon_action(&is_menu, Button::Right), IconAction::OpenMenu);
    // ItemIsMenu without a dbusmenu: the item shows its own menu.
    assert_eq!(
        icon_action(&bar_item(None, true), Button::Left),
        IconAction::ContextMenu
    );
    // A menu whose entries are all hidden is no menu.
    let hidden = MenuNode {
        visible: false,
        ..node(1, "Hidden", vec![])
    };
    let empty = bar_item(Some(node(0, "", vec![hidden])), false);
    assert_eq!(icon_action(&empty, Button::Right), IconAction::ContextMenu);
}

#[test]
fn button_codes() {
    assert_eq!(Button::from_code(0x110), Some(Button::Left));
    assert_eq!(Button::from_code(0x111), Some(Button::Right));
    assert_eq!(Button::from_code(0x112), Some(Button::Middle));
    assert_eq!(Button::from_code(0x113), None);
}

#[test]
fn menu_entries_strip_hidden_entries_and_collapse_separators() {
    let hidden = MenuNode {
        visible: false,
        ..node(5, "Hidden", vec![])
    };
    let disabled = MenuNode {
        enabled: false,
        ..node(6, "Disabled", vec![])
    };
    let check = MenuNode {
        toggle: Toggle::Check(true),
        ..node(7, "Check", vec![])
    };
    let radio = MenuNode {
        toggle: Toggle::Radio(false),
        ..node(8, "Radio", vec![])
    };
    let root = node(
        0,
        "",
        vec![
            separator(1),
            node(2, "Open", vec![]),
            separator(3),
            hidden,
            separator(4),
            disabled,
            check,
            radio,
            node(9, "More", vec![node(10, "Inner", vec![])]),
            separator(11),
        ],
    );
    let (entries, ids) = menu_entries(&root);
    assert_eq!(ids, vec![2, 3, 6, 7, 8, 9]);
    let kinds: Vec<MenuEntryKind> = entries.iter().map(|e| e.kind).collect();
    assert_eq!(
        kinds,
        vec![
            MenuEntryKind::Normal,
            MenuEntryKind::Separator,
            MenuEntryKind::Normal,
            MenuEntryKind::Check(true),
            MenuEntryKind::Radio(false),
            MenuEntryKind::Normal,
        ]
    );
    assert!(!entries[2].enabled);
    assert_eq!(entries[3].label, "Check");
    assert!(entries[5].submenu);
    assert!(!entries[0].submenu);
}

#[test]
fn an_entry_whose_children_are_hidden_has_no_submenu() {
    let hidden = MenuNode {
        visible: false,
        ..node(2, "Hidden", vec![])
    };
    let root = node(0, "", vec![node(1, "Leaf", vec![hidden])]);
    let (entries, _) = menu_entries(&root);
    assert!(!entries[0].submenu);
}

#[test]
fn find_searches_the_whole_tree() {
    let root = node(0, "", vec![node(1, "A", vec![node(2, "B", vec![])])]);
    assert_eq!(find(&root, 2).map(|n| n.label.as_str()), Some("B"));
    assert_eq!(find(&root, 0).map(|n| n.id), Some(0));
    assert!(find(&root, 3).is_none());
}

const SCREEN: Rect = Rect::new(0, 0, 1920, 1080);

#[test]
fn the_root_menu_hangs_below_a_top_bar_inside_the_screen() {
    let anchor = Anchor {
        x: 100,
        bar: Rect::new(0, 0, 1920, 24),
        below: true,
        area: SCREEN,
    };
    assert_eq!(
        place_root((200, 100), &anchor),
        Rect::new(100, 24, 200, 100)
    );
    // Near the right edge it moves left to stay on screen.
    let right = Anchor { x: 1850, ..anchor };
    assert_eq!(
        place_root((200, 100), &right),
        Rect::new(1720, 24, 200, 100)
    );
}

#[test]
fn the_root_menu_sits_above_a_bottom_bar() {
    let anchor = Anchor {
        x: 1900,
        bar: Rect::new(0, 1056, 1920, 24),
        below: false,
        area: SCREEN,
    };
    assert_eq!(
        place_root((200, 100), &anchor),
        Rect::new(1720, 956, 200, 100)
    );
    // Taller than the screen: clamped to its top.
    assert_eq!(place_root((200, 2000), &anchor).y, 0);
}

#[test]
fn submenus_open_right_of_their_entry_or_else_left() {
    let parent = Rect::new(100, 24, 200, 100);
    assert_eq!(
        place_submenu((150, 80), parent, 30, SCREEN),
        Rect::new(300, 54, 150, 80)
    );
    let parent = Rect::new(1720, 24, 200, 100);
    assert_eq!(
        place_submenu((150, 80), parent, 30, SCREEN),
        Rect::new(1570, 54, 150, 80)
    );
    // Near the bottom it moves up.
    let parent = Rect::new(100, 1000, 200, 80);
    assert_eq!(place_submenu((150, 80), parent, 60, SCREEN).y, 1000);
}

fn fonts() -> Fonts {
    Fonts::embedded()
}

fn tree() -> MenuNode {
    node(
        0,
        "",
        vec![
            node(1, "Open", vec![]),
            separator(2),
            node(
                3,
                "More",
                vec![node(4, "Inner", vec![]), node(5, "Other", vec![])],
            ),
            MenuNode {
                enabled: false,
                ..node(6, "Off", vec![])
            },
            node(7, "Quit", vec![]),
        ],
    )
}

fn open_menu(area: Rect) -> OpenMenu {
    let anchor = Anchor {
        x: 100,
        bar: Rect::new(0, 0, 1920, 24),
        below: true,
        area,
    };
    let mut menu = OpenMenu::new("item".to_owned(), anchor, 0);
    assert!(menu.refresh(&tree(), &MenuStyle::default(), &fonts()));
    menu
}

#[test]
fn refresh_builds_the_root_level() {
    let menu = open_menu(SCREEN);
    assert_eq!(menu.levels.len(), 1);
    let level = &menu.levels[0];
    assert_eq!(level.ids, vec![1, 2, 3, 6, 7]);
    let (w, h) = menu_size(&level.view, &MenuStyle::default(), &fonts());
    assert_eq!(
        level.rect,
        Rect::new(100, 24, w.cast_signed(), h.cast_signed())
    );
}

#[test]
fn clicking_entries_opens_submenus_and_activates_leaves() {
    let mut menu = open_menu(SCREEN);
    assert_eq!(menu.click(0, 2), Outcome::AboutToShow(3));
    assert!(menu.refresh(&tree(), &MenuStyle::default(), &fonts()));
    assert_eq!(menu.levels.len(), 2);
    assert_eq!(menu.levels[1].ids, vec![4, 5]);
    let parent = menu.levels[0].rect;
    assert_eq!(menu.levels[1].rect.x, parent.x + parent.w);
    assert_eq!(menu.click(1, 1), Outcome::Activate(5));
    // Clicking the open submenu's entry again closes it.
    assert_eq!(menu.click(0, 2), Outcome::Stay);
    assert_eq!(menu.levels.len(), 1);
    // Separators and disabled entries do nothing.
    assert_eq!(menu.click(0, 1), Outcome::Stay);
    assert_eq!(menu.click(0, 3), Outcome::Stay);
    assert_eq!(menu.click(0, 4), Outcome::Activate(7));
}

#[test]
fn keys_navigate_the_deepest_level() {
    let mut menu = open_menu(SCREEN);
    let style = MenuStyle::default();
    assert_eq!(menu.key(PromptKey::Down), Outcome::Stay);
    assert_eq!(menu.levels[0].selected, Some(0));
    // Skips the separator.
    menu.key(PromptKey::Char('j'));
    assert_eq!(menu.levels[0].selected, Some(2));
    // Skips the disabled entry.
    menu.key(PromptKey::Down);
    assert_eq!(menu.levels[0].selected, Some(4));
    // Wraps.
    menu.key(PromptKey::Down);
    assert_eq!(menu.levels[0].selected, Some(0));
    menu.key(PromptKey::Up);
    assert_eq!(menu.levels[0].selected, Some(4));
    menu.key(PromptKey::Char('k'));
    assert_eq!(menu.levels[0].selected, Some(2));
    assert_eq!(menu.key(PromptKey::Right), Outcome::AboutToShow(3));
    assert!(menu.refresh(&tree(), &style, &fonts()));
    menu.key(PromptKey::Down);
    assert_eq!(menu.levels[1].selected, Some(0));
    assert_eq!(menu.key(PromptKey::Return), Outcome::Activate(4));
    assert_eq!(menu.key(PromptKey::Left), Outcome::Stay);
    assert_eq!(menu.levels.len(), 1);
    assert_eq!(menu.key(PromptKey::Return), Outcome::AboutToShow(3));
    assert!(menu.refresh(&tree(), &style, &fonts()));
    assert_eq!(menu.key(PromptKey::Escape), Outcome::Stay);
    assert_eq!(menu.levels.len(), 1);
    assert_eq!(menu.key(PromptKey::Escape), Outcome::Close);
}

#[test]
fn refresh_follows_layout_changes() {
    let mut menu = open_menu(SCREEN);
    menu.click(0, 2);
    menu.hover(0, 4);
    assert!(menu.refresh(&tree(), &MenuStyle::default(), &fonts()));
    // The submenu's entry goes away: its level closes, the selection is
    // kept where it is still valid.
    let smaller = node(
        0,
        "",
        vec![node(1, "Open", vec![]), node(7, "Quit", vec![])],
    );
    assert!(menu.refresh(&smaller, &MenuStyle::default(), &fonts()));
    assert_eq!(menu.levels.len(), 1);
    assert_eq!(menu.levels[0].ids, vec![1, 7]);
    assert_eq!(menu.levels[0].selected, None);
    // Nothing left to show: the menu closes.
    assert!(!menu.refresh(&node(0, "", vec![]), &MenuStyle::default(), &fonts()));
}

#[test]
fn long_menus_are_truncated_to_the_screen() {
    let style = MenuStyle::default();
    let mut menu = open_menu(Rect::new(0, 0, 1920, 40));
    assert!(menu.refresh(&tree(), &style, &fonts()));
    let level = &menu.levels[0];
    assert!(level.rect.h <= 40, "{:?}", level.rect);
    assert_eq!(level.ids, vec![1]);
    assert_eq!(level.ids.len(), level.view.entries.len());
}
