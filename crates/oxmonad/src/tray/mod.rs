//! The system tray: a `StatusNotifierItem` host on the session bus.
//!
//! Implements the freedesktop/KDE
//! [StatusNotifierItem](https://www.freedesktop.org/wiki/Specifications/StatusNotifierItem/)
//! spec plus `com.canonical.dbusmenu` for the items' menus. All D-Bus work
//! happens on a dedicated `oxmonad-tray` thread; the main loop only
//! receives [`TrayEvent`]s over a calloop channel and sends commands
//! through a non-blocking channel, so it never waits on D-Bus.
//!
//! When no other process owns `org.kde.StatusNotifierWatcher`, the tray
//! is the watcher too; otherwise (waybar, a desktop shell…) it registers
//! as a host with the existing watcher and follows its signals. If that
//! watcher goes away, the tray takes the name over.

mod icon;
mod item;
mod menu;
mod service;
mod theme;
mod watcher;

#[cfg(test)]
mod tests;

use std::{
    io,
    thread::{
        self,
        JoinHandle,
    },
};

use calloop::channel::Channel;

/// What the tray thread reports to the main loop.
#[derive(Clone, Debug, PartialEq)]
pub enum TrayEvent {
    /// The full current list of items, in registration order.
    Items(Vec<TrayItem>),
}

/// One `StatusNotifierItem` as the bar should show it.
#[derive(Clone, Debug, PartialEq)]
pub struct TrayItem {
    /// Stable key: the registered bus name followed by the object path.
    pub id: String,
    /// `Title`, or the item's `Id` when it has no title.
    pub title: String,
    /// The item's `Status`.
    pub status: ItemStatus,
    /// The icon for the current status (the attention icon while
    /// [`ItemStatus::NeedsAttention`], when the item has one).
    pub icon: Option<TrayIcon>,
    /// `ItemIsMenu`: a left click should open the menu too.
    pub item_is_menu: bool,
    /// Root of the item's dbusmenu layout, if it exports one.
    pub menu: Option<MenuNode>,
}

/// `Status` of a `StatusNotifierItem`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ItemStatus {
    /// Not important right now; a bar may hide it.
    Passive,
    /// Normal.
    #[default]
    Active,
    /// Wants the user's attention.
    NeedsAttention,
}

/// A decoded icon.
#[derive(Clone, PartialEq, Eq)]
pub struct TrayIcon {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Premultiplied ARGB8888 little-endian (bytes B, G, R, A), the
    /// layout of `wl_shm` `argb8888`; `width * height * 4` bytes.
    pub bgra: Vec<u8>,
}

impl std::fmt::Debug for TrayIcon {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TrayIcon")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("bgra", &format_args!("[{} bytes]", self.bgra.len()))
            .finish()
    }
}

/// One entry of a dbusmenu layout.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuNode {
    /// The dbusmenu item id (`0` is the root).
    pub id: i32,
    /// The label with its mnemonics stripped (`_` removed, `__` → `_`).
    pub label: String,
    /// Normal entry or separator.
    pub kind: MenuKind,
    /// Check box or radio button state.
    pub toggle: Toggle,
    /// Whether the entry can be activated.
    pub enabled: bool,
    /// Whether the entry is shown at all.
    pub visible: bool,
    /// Submenu entries.
    pub children: Vec<MenuNode>,
}

/// The dbusmenu `type` of an entry.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MenuKind {
    /// A normal, clickable entry.
    #[default]
    Normal,
    /// A separator line.
    Separator,
}

/// The dbusmenu `toggle-type` and `toggle-state` of an entry.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Toggle {
    /// No toggle.
    #[default]
    None,
    /// A check box; `true` when checked.
    Check(bool),
    /// A radio button; `true` when selected.
    Radio(bool),
}

/// Requests from the main loop to the tray thread.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Command {
    Activate {
        item: String,
        x: i32,
        y: i32,
    },
    SecondaryActivate {
        item: String,
        x: i32,
        y: i32,
    },
    ContextMenu {
        item: String,
        x: i32,
        y: i32,
    },
    Scroll {
        item: String,
        delta: i32,
        horizontal: bool,
    },
    MenuAboutToShow {
        item: String,
        id: i32,
    },
    MenuClicked {
        item: String,
        id: i32,
    },
}

/// Handle to the tray thread. Dropping it stops the thread.
#[derive(Debug)]
pub struct Tray {
    messages: async_channel::Sender<service::Message>,
    thread: Option<JoinHandle<()>>,
}

impl Tray {
    /// Starts the tray thread on the session bus; events arrive on the
    /// returned calloop channel. `icon_size` is the logical icon size;
    /// icons are prepared for twice that (scale 2).
    ///
    /// Returns at once: the thread connects to the bus on its own. Without
    /// a session bus it logs a warning and exits, and no events arrive.
    /// Fails only if the thread cannot be spawned.
    pub fn start(icon_size: u32) -> io::Result<(Self, Channel<TrayEvent>)> {
        Self::start_on(icon_size, None)
    }

    /// [`Tray::start`] on the bus at `address`, or the session bus.
    fn start_on(icon_size: u32, address: Option<String>) -> io::Result<(Self, Channel<TrayEvent>)> {
        let (events, channel) = calloop::channel::channel();
        let (messages, inbox) = async_channel::unbounded();
        let config = service::Config {
            address,
            icon_size,
            inbox,
            outbox: messages.clone(),
            events,
        };
        let thread = thread::Builder::new()
            .name("oxmonad-tray".to_owned())
            .spawn(move || service::run(config))?;
        Ok((
            Self {
                messages,
                thread: Some(thread),
            },
            channel,
        ))
    }

    fn send(&self, command: Command) {
        if self
            .messages
            .try_send(service::Message::Command(command))
            .is_err()
        {
            tracing::debug!("tray thread is gone");
        }
    }

    /// Left click: `Activate(x, y)`.
    pub fn activate(&self, item: &str, x: i32, y: i32) {
        self.send(Command::Activate {
            item: item.to_owned(),
            x,
            y,
        });
    }

    /// Middle click: `SecondaryActivate(x, y)`.
    pub fn secondary_activate(&self, item: &str, x: i32, y: i32) {
        self.send(Command::SecondaryActivate {
            item: item.to_owned(),
            x,
            y,
        });
    }

    /// Right click on an item without a dbusmenu: `ContextMenu(x, y)`.
    pub fn context_menu(&self, item: &str, x: i32, y: i32) {
        self.send(Command::ContextMenu {
            item: item.to_owned(),
            x,
            y,
        });
    }

    /// Scroll wheel: `Scroll(delta, "horizontal" | "vertical")`.
    pub fn scroll(&self, item: &str, delta: i32, horizontal: bool) {
        self.send(Command::Scroll {
            item: item.to_owned(),
            delta,
            horizontal,
        });
    }

    /// The (sub)menu `id` is about to be shown: dbusmenu `AboutToShow`.
    /// A changed layout arrives as a new [`TrayEvent::Items`].
    pub fn menu_about_to_show(&self, item: &str, id: i32) {
        self.send(Command::MenuAboutToShow {
            item: item.to_owned(),
            id,
        });
    }

    /// The menu entry `id` was clicked: dbusmenu `Event(id, "clicked")`.
    pub fn menu_clicked(&self, item: &str, id: i32) {
        self.send(Command::MenuClicked {
            item: item.to_owned(),
            id,
        });
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        // Never join: the thread may be waiting on a (timed) D-Bus call.
        // It exits once it sees the closed channel.
        self.messages.close();
        drop(self.thread.take());
    }
}
