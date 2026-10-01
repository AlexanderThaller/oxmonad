//! The system tray in the UI: item icons in the bar and an item's popup
//! menu.
//!
//! Pure: no Wayland and no D-Bus. The binary feeds it the
//! [`TrayItem`]s the [`tray`](crate::tray) thread reports and turns its
//! answers into tray calls and shell surfaces.

use oxmonad_core::{
    Rect,
    prompt::PromptKey,
};
use oxmonad_ui::{
    Fonts,
    Icon,
    MenuEntry,
    MenuEntryKind,
    MenuStyle,
    MenuView,
    Segment,
    menu_entry_rects,
    menu_size,
};

use crate::tray::{
    ItemStatus,
    MenuKind,
    MenuNode,
    Toggle,
    TrayItem,
};

#[cfg(test)]
mod tests;

/// A tray item as the bar shows it: a [`TrayItem`] with its icon
/// converted once, when it arrives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BarItem {
    /// The item's stable key, also the action of its bar segment.
    pub id: String,
    /// The item's title.
    pub title: String,
    /// The item's status; passive items are not shown.
    pub status: ItemStatus,
    /// The icon; items without one are not shown.
    pub icon: Option<Icon>,
    /// A left click opens the menu too.
    pub item_is_menu: bool,
    /// The item's dbusmenu.
    pub menu: Option<MenuNode>,
}

impl BarItem {
    /// Converts `item`.
    #[must_use]
    pub fn new(item: TrayItem) -> Self {
        let icon = item
            .icon
            .and_then(|icon| Icon::from_premultiplied_bgra(icon.width, icon.height, icon.bgra));
        Self {
            id: item.id,
            title: item.title,
            status: item.status,
            icon,
            item_is_menu: item.item_is_menu,
            menu: item.menu,
        }
    }

    /// Whether the item has a dbusmenu with something to show.
    #[must_use]
    pub fn has_menu(&self) -> bool {
        self.menu.as_ref().is_some_and(has_visible_children)
    }

    /// Whether the bar shows the item.
    #[must_use]
    pub fn is_shown(&self) -> bool {
        self.status != ItemStatus::Passive && self.icon.is_some()
    }
}

/// Converts the items the tray thread reported.
#[must_use]
pub fn bar_items(items: Vec<TrayItem>) -> Vec<BarItem> {
    items
        .into_iter()
        .map(|item| {
            let item = BarItem::new(item);
            if item.icon.is_none() {
                tracing::debug!(id = %item.id, title = %item.title, "tray item without an icon");
            }
            item
        })
        .collect()
}

/// The bar segments of the tray: one icon per shown item, in order, each
/// with the item id as its action.
#[must_use]
pub fn segments(items: &[BarItem]) -> Vec<Segment> {
    items
        .iter()
        .filter(|item| item.is_shown())
        .filter_map(|item| {
            let icon = item.icon.clone()?;
            Some(Segment::with_icon(icon).action(item.id.clone()))
        })
        .collect()
}

/// A pointer button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    /// `BTN_LEFT`.
    Left,
    /// `BTN_MIDDLE`.
    Middle,
    /// `BTN_RIGHT`.
    Right,
}

impl Button {
    /// The button of a Linux evdev code, if it is one of the three.
    #[must_use]
    pub const fn from_code(code: u32) -> Option<Self> {
        match code {
            0x110 => Some(Self::Left),
            0x111 => Some(Self::Right),
            0x112 => Some(Self::Middle),
            _ => None,
        }
    }
}

/// What a click on a tray icon does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IconAction {
    /// `Activate(x, y)`.
    Activate,
    /// `SecondaryActivate(x, y)`.
    SecondaryActivate,
    /// `ContextMenu(x, y)`: the item shows its own menu.
    ContextMenu,
    /// Open the item's dbusmenu as a popup.
    OpenMenu,
}

/// What clicking `button` on `item`'s icon does: left activates (or opens
/// the menu of an `ItemIsMenu` item), middle is the secondary activation,
/// right opens the dbusmenu or asks the item for its context menu.
#[must_use]
pub fn icon_action(item: &BarItem, button: Button) -> IconAction {
    match button {
        Button::Left if item.item_is_menu && item.has_menu() => IconAction::OpenMenu,
        Button::Left if item.item_is_menu => IconAction::ContextMenu,
        Button::Left => IconAction::Activate,
        Button::Middle => IconAction::SecondaryActivate,
        Button::Right if item.has_menu() => IconAction::OpenMenu,
        Button::Right => IconAction::ContextMenu,
    }
}

fn has_visible_children(node: &MenuNode) -> bool {
    node.children.iter().any(|child| child.visible)
}

/// The rows of `node`'s submenu and their dbusmenu ids: invisible entries
/// dropped, separators collapsed (none leading, trailing or doubled).
#[must_use]
pub fn menu_entries(node: &MenuNode) -> (Vec<MenuEntry>, Vec<i32>) {
    let mut entries: Vec<MenuEntry> = Vec::new();
    let mut ids = Vec::new();
    for child in node.children.iter().filter(|child| child.visible) {
        let entry = match child.kind {
            MenuKind::Separator => {
                if entries
                    .last()
                    .is_none_or(|last| last.kind == MenuEntryKind::Separator)
                {
                    continue;
                }
                MenuEntry::separator()
            }
            MenuKind::Normal => MenuEntry {
                label: child.label.clone(),
                kind: match child.toggle {
                    Toggle::None => MenuEntryKind::Normal,
                    Toggle::Check(on) => MenuEntryKind::Check(on),
                    Toggle::Radio(on) => MenuEntryKind::Radio(on),
                },
                enabled: child.enabled,
                submenu: has_visible_children(child),
            },
        };
        entries.push(entry);
        ids.push(child.id);
    }
    trim_separators(&mut entries, &mut ids);
    (entries, ids)
}

/// Drops trailing separators.
fn trim_separators(entries: &mut Vec<MenuEntry>, ids: &mut Vec<i32>) {
    while entries
        .last()
        .is_some_and(|last| last.kind == MenuEntryKind::Separator)
    {
        entries.pop();
        ids.pop();
    }
}

/// The entry `id` anywhere in the tree under (and including) `node`.
#[must_use]
pub fn find(node: &MenuNode, id: i32) -> Option<&MenuNode> {
    if node.id == id {
        return Some(node);
    }
    node.children.iter().find_map(|child| find(child, id))
}

/// Where a menu hangs: under (or over) a tray icon in the bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Anchor {
    /// The icon's left edge, global logical x.
    pub x: i32,
    /// The bar's rectangle.
    pub bar: Rect,
    /// Whether the menu opens below the bar (a top bar) or above it.
    pub below: bool,
    /// The area menus must stay in: the bar output's usable area.
    pub area: Rect,
}

/// Moves `rect` into `area` (its top left corner wins when it is larger).
fn clamp_into(rect: Rect, area: Rect) -> Rect {
    let x = rect.x.min(area.x + area.w - rect.w).max(area.x);
    let y = rect.y.min(area.y + area.h - rect.h).max(area.y);
    Rect::new(x, y, rect.w, rect.h)
}

/// Where a root menu of logical `size` goes: at the icon's left edge,
/// against the bar, inside the anchor's area.
#[must_use]
pub fn place_root((w, h): (i32, i32), anchor: &Anchor) -> Rect {
    let y = if anchor.below {
        anchor.bar.y + anchor.bar.h
    } else {
        anchor.bar.y - h
    };
    clamp_into(Rect::new(anchor.x, y, w, h), anchor.area)
}

/// Where a submenu of logical `size` goes: right of its `parent` menu with
/// its top at `top` (relative to the parent), or left of the parent when
/// it does not fit on the right; inside `area`.
#[must_use]
pub fn place_submenu((w, h): (i32, i32), parent: Rect, top: i32, area: Rect) -> Rect {
    let right = parent.x + parent.w;
    let x = if right + w > area.x + area.w {
        parent.x - w
    } else {
        right
    };
    clamp_into(Rect::new(x, parent.y + top, w, h), area)
}

/// One open (sub)menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Level {
    /// The dbusmenu id of the entry whose children this level shows.
    pub node: i32,
    /// The highlighted row.
    pub selected: Option<usize>,
    /// What is drawn (set by [`OpenMenu::refresh`]).
    pub view: MenuView,
    /// The dbusmenu id of each row.
    pub ids: Vec<i32>,
    /// Where the level is drawn, global logical.
    pub rect: Rect,
}

impl Level {
    fn new(node: i32) -> Self {
        Self {
            node,
            selected: None,
            view: MenuView::default(),
            ids: Vec::new(),
            rect: Rect::new(0, 0, 0, 0),
        }
    }

    fn is_selectable(&self, index: usize) -> bool {
        self.view
            .entries
            .get(index)
            .is_some_and(MenuEntry::is_selectable)
    }
}

/// What an [`OpenMenu`] wants done after an input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Nothing beyond redrawing.
    Stay,
    /// A submenu opened: dbusmenu `AboutToShow(id)`.
    AboutToShow(i32),
    /// An entry was chosen: dbusmenu `Event(id, "clicked")`, then close.
    Activate(i32),
    /// Close the menu.
    Close,
}

/// A tray item's popup menu and its open submenus (cascading to the
/// side).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenMenu {
    /// The tray item.
    pub item: String,
    /// Where the root menu hangs.
    pub anchor: Anchor,
    /// The root menu first, then each open submenu.
    pub levels: Vec<Level>,
}

impl OpenMenu {
    /// A menu showing the children of `root` (the dbusmenu root id); drawn
    /// once [`Self::refresh`]ed.
    #[must_use]
    pub fn new(item: String, anchor: Anchor, root: i32) -> Self {
        Self {
            item,
            anchor,
            levels: vec![Level::new(root)],
        }
    }

    /// Rebuilds every level from the item's current menu `root`: closes
    /// submenus whose entry is gone, drops selections that no longer
    /// point at a selectable row, truncates menus taller than the anchor's
    /// area and places them. `false` when there is nothing left to show.
    pub fn refresh(&mut self, root: &MenuNode, style: &MenuStyle, fonts: &Fonts) -> bool {
        let area = self.anchor.area;
        let mut kept = 0;
        for index in 0..self.levels.len() {
            let node_id = self.levels[index].node;
            let node = if index == 0 {
                Some(root)
            } else {
                find(root, node_id)
            };
            let Some(node) = node.filter(|node| has_visible_children(node)) else {
                break;
            };
            // A submenu stays open only while its entry is shown.
            let opener = match index.checked_sub(1) {
                Some(parent) => {
                    let parent = &self.levels[parent];
                    let Some(row) = parent.ids.iter().position(|&id| id == node_id) else {
                        break;
                    };
                    Some((
                        parent.rect,
                        menu_entry_rects(&parent.view, style, fonts)[row].1,
                    ))
                }
                None => None,
            };
            let (mut entries, mut ids) = menu_entries(node);
            let level = &mut self.levels[index];
            let mut view = MenuView {
                entries: Vec::new(),
                selected: None,
            };
            loop {
                view.entries.clone_from(&entries);
                let (_, height) = menu_size(&view, style, fonts);
                if entries.is_empty() || i64::from(height) <= i64::from(area.h) {
                    break;
                }
                entries.pop();
                ids.pop();
                trim_separators(&mut entries, &mut ids);
            }
            if level
                .selected
                .is_some_and(|row| !view.entries.get(row).is_some_and(MenuEntry::is_selectable))
            {
                level.selected = None;
            }
            view.selected = level.selected;
            let (w, h) = menu_size(&view, style, fonts);
            let size = (
                i32::try_from(w).unwrap_or(i32::MAX),
                i32::try_from(h).unwrap_or(i32::MAX),
            );
            level.rect = match opener {
                Some((parent, top)) => {
                    let border = i32::try_from(style.border_width).unwrap_or(0);
                    place_submenu(size, parent, top - border, area)
                }
                None => place_root(size, &self.anchor),
            };
            level.view = view;
            level.ids = ids;
            kept = index + 1;
        }
        self.levels.truncate(kept);
        kept > 0
    }

    /// The pointer is over row `row` of level `level`: highlights it.
    pub fn hover(&mut self, level: usize, row: usize) {
        if let Some(level) = self.levels.get_mut(level) {
            level.selected = Some(row);
            level.view.selected = Some(row);
        }
    }

    /// Row `row` of level `level` was clicked: opens or closes its
    /// submenu, or chooses it.
    pub fn click(&mut self, level: usize, row: usize) -> Outcome {
        let Some(current) = self.levels.get(level) else {
            return Outcome::Stay;
        };
        let (Some(&id), Some(entry)) = (current.ids.get(row), current.view.entries.get(row)) else {
            return Outcome::Stay;
        };
        if !entry.is_selectable() {
            return Outcome::Stay;
        }
        if !entry.submenu {
            return Outcome::Activate(id);
        }
        let open = self
            .levels
            .get(level + 1)
            .is_some_and(|next| next.node == id);
        self.levels.truncate(level + 1);
        self.hover(level, row);
        if open {
            Outcome::Stay
        } else {
            self.levels.push(Level::new(id));
            Outcome::AboutToShow(id)
        }
    }

    /// A key typed while the menu has the keyboard; it acts on the deepest
    /// open level: `j`/`k`/arrows/`Tab` move, `Return`/`space` choose,
    /// `Right`/`l` open a submenu, `Left`/`h` close it, `Escape` closes a
    /// submenu or the menu.
    pub fn key(&mut self, key: PromptKey) -> Outcome {
        let Some(deepest) = self.levels.len().checked_sub(1) else {
            return Outcome::Close;
        };
        let selected = self.levels[deepest].selected;
        match key {
            PromptKey::Escape if deepest == 0 => Outcome::Close,
            PromptKey::Escape | PromptKey::Left | PromptKey::Char('h') => {
                if deepest > 0 {
                    self.levels.pop();
                }
                Outcome::Stay
            }
            PromptKey::Down | PromptKey::Tab | PromptKey::Char('j') => {
                self.step(deepest, true);
                Outcome::Stay
            }
            PromptKey::Up | PromptKey::BackTab | PromptKey::Char('k') => {
                self.step(deepest, false);
                Outcome::Stay
            }
            PromptKey::Right | PromptKey::Char('l') => match selected {
                Some(row) if self.levels[deepest].view.entries[row].submenu => {
                    self.click(deepest, row)
                }
                _ => Outcome::Stay,
            },
            PromptKey::Return | PromptKey::Char(' ') => match selected {
                Some(row) => self.click(deepest, row),
                None => Outcome::Stay,
            },
            PromptKey::Char(_)
            | PromptKey::Backspace
            | PromptKey::Delete
            | PromptKey::PageUp
            | PromptKey::PageDown
            | PromptKey::Home
            | PromptKey::End
            | PromptKey::DeleteWord
            | PromptKey::KillToEnd
            | PromptKey::KillToStart => Outcome::Stay,
        }
    }

    /// Moves level `index`'s selection to the next (or previous)
    /// selectable row, wrapping around.
    fn step(&mut self, index: usize, forward: bool) {
        let level = &self.levels[index];
        let len = level.view.entries.len();
        if len == 0 {
            return;
        }
        let start = match (level.selected, forward) {
            (Some(row), _) => row,
            (None, true) => len - 1,
            (None, false) => 0,
        };
        let next = (1..=len)
            .map(|offset| {
                if forward {
                    (start + offset) % len
                } else {
                    (start + len - offset % len) % len
                }
            })
            .find(|&row| level.is_selectable(row));
        if let Some(row) = next {
            self.hover(index, row);
        }
    }
}
