//! One `StatusNotifierItem`: its properties and the task that follows it.

use std::{
    collections::HashMap,
    future::Future,
    time::Duration,
};

use async_io::Timer;
use futures_lite::{
    FutureExt,
    StreamExt,
    future,
};
use zbus::{
    Connection,
    MatchRule,
    Message,
    MessageStream,
    export::serde::Serialize,
    message,
    zvariant::{
        DynamicType,
        OwnedValue,
        Value,
    },
};

use super::{
    ItemStatus,
    MenuNode,
    icon::RawPixmap,
    menu::{
        self,
        LayoutReply,
        unwrap_variant,
    },
    service,
    watcher::Registration,
};

/// The `StatusNotifierItem` interface.
pub const INTERFACE: &str = "org.kde.StatusNotifierItem";

/// The properties read from an item.
const PROPERTIES: [&str; 10] = [
    "Id",
    "Title",
    "Status",
    "IconName",
    "IconPixmap",
    "AttentionIconName",
    "AttentionIconPixmap",
    "IconThemePath",
    "ItemIsMenu",
    "Menu",
];

/// Longest wait for any call to an item.
pub const CALL_TIMEOUT: Duration = Duration::from_secs(2);

/// Signal bursts (an icon and a status change…) are collected this long
/// before the item is read again.
const SIGNAL_DEBOUNCE: Duration = Duration::from_millis(30);

/// How often reading a new item is retried: some register before they
/// export their object.
const INITIAL_ATTEMPTS: u32 = 3;
const RETRY_DELAY: Duration = Duration::from_millis(500);

/// The `StatusNotifierItem` properties the tray uses; all optional.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ItemProps {
    pub id: Option<String>,
    pub title: Option<String>,
    pub status: ItemStatus,
    pub icon_name: Option<String>,
    pub icon_pixmaps: Vec<RawPixmap>,
    pub attention_icon_name: Option<String>,
    pub attention_pixmaps: Vec<RawPixmap>,
    pub theme_path: Option<String>,
    pub item_is_menu: bool,
    pub menu: Option<String>,
}

/// A non-empty string or object path.
fn string(value: &Value<'_>) -> Option<String> {
    let value = unwrap_variant(value);
    let text = if let Value::Str(s) = value {
        Some(s.as_str())
    } else if let Value::ObjectPath(p) = value {
        Some(p.as_str())
    } else {
        None
    };
    text.filter(|s| !s.is_empty()).map(str::to_owned)
}

/// An `a(iiay)` icon pixmap list; malformed entries are skipped.
pub fn pixmaps(value: &Value<'_>) -> Vec<RawPixmap> {
    let Value::Array(array) = unwrap_variant(value) else {
        return Vec::new();
    };
    array
        .inner()
        .iter()
        .filter_map(|entry| {
            let Value::Structure(entry) = unwrap_variant(entry) else {
                return None;
            };
            let [Value::I32(width), Value::I32(height), Value::Array(bytes)] = entry.fields()
            else {
                return None;
            };
            let argb = bytes
                .inner()
                .iter()
                .map(|b| if let Value::U8(b) = b { Some(*b) } else { None })
                .collect::<Option<Vec<u8>>>()?;
            Some(RawPixmap {
                width: *width,
                height: *height,
                argb,
            })
        })
        .collect()
}

/// A valid dbusmenu object path (not `/` or `/NO_DBUSMENU`).
fn menu_path(value: &Value<'_>) -> Option<String> {
    string(value).filter(|p| p.starts_with('/') && p != "/" && p != "/NO_DBUSMENU")
}

impl ItemProps {
    /// Reads the known properties from `(name, value)` pairs; anything
    /// missing or of the wrong type is left at its default.
    pub fn from_props<'a>(props: impl Iterator<Item = (&'a str, &'a Value<'a>)>) -> Self {
        let mut item = Self::default();
        for (name, value) in props {
            match name {
                "Id" => item.id = string(value),
                "Title" => item.title = string(value),
                "Status" => {
                    item.status = match string(value).as_deref() {
                        Some("Passive") => ItemStatus::Passive,
                        Some("NeedsAttention") => ItemStatus::NeedsAttention,
                        _ => ItemStatus::Active,
                    }
                }
                "IconName" => item.icon_name = string(value),
                "IconPixmap" => item.icon_pixmaps = pixmaps(value),
                "AttentionIconName" => item.attention_icon_name = string(value),
                "AttentionIconPixmap" => item.attention_pixmaps = pixmaps(value),
                "IconThemePath" => item.theme_path = string(value),
                "ItemIsMenu" => {
                    item.item_is_menu = matches!(unwrap_variant(value), Value::Bool(true));
                }
                "Menu" => item.menu = menu_path(value),
                _ => {}
            }
        }
        item
    }

    fn from_map(map: &HashMap<String, OwnedValue>) -> Self {
        Self::from_props(map.iter().map(|(name, value)| (name.as_str(), &**value)))
    }
}

/// Runs `future` for at most `limit`.
pub async fn timeout<T>(limit: Duration, future: impl Future<Output = T>) -> Option<T> {
    async { Some(future.await) }
        .or(async {
            Timer::after(limit).await;
            None
        })
        .await
}

/// Calls `method` with [`CALL_TIMEOUT`]; failures are logged at debug.
pub async fn call<B>(
    conn: &Connection,
    destination: &str,
    path: &str,
    interface: &str,
    method: &str,
    body: &B,
) -> Option<Message>
where
    B: Serialize + DynamicType,
{
    let reply = conn.call_method(Some(destination), path, Some(interface), method, body);
    match timeout(CALL_TIMEOUT, reply).await {
        Some(Ok(reply)) => Some(reply),
        Some(Err(err)) => {
            tracing::debug!(%err, destination, path, method, "tray call failed");
            None
        }
        None => {
            tracing::debug!(destination, path, method, "tray call timed out");
            None
        }
    }
}

/// The unique name owning `service`.
async fn owner(conn: &Connection, service: &str) -> Option<String> {
    if service.starts_with(':') {
        return Some(service.to_owned());
    }
    let reply = call(
        conn,
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
        "GetNameOwner",
        &service,
    )
    .await?;
    reply.body().deserialize::<String>().ok()
}

/// Reads the item's properties: `GetAll`, or one by one if that fails.
async fn fetch_props(conn: &Connection, owner: &str, path: &str) -> Option<ItemProps> {
    const PROPS: &str = "org.freedesktop.DBus.Properties";
    if let Some(reply) = call(conn, owner, path, PROPS, "GetAll", &INTERFACE).await {
        match reply.body().deserialize::<HashMap<String, OwnedValue>>() {
            Ok(map) => return Some(ItemProps::from_map(&map)),
            Err(err) => tracing::debug!(%err, owner, path, "tray item properties"),
        }
    }
    let mut map = HashMap::new();
    for name in PROPERTIES {
        if let Some(reply) = call(conn, owner, path, PROPS, "Get", &(INTERFACE, name)).await
            && let Ok(value) = reply.body().deserialize::<OwnedValue>()
        {
            map.insert(name.to_owned(), value);
        }
    }
    (!map.is_empty()).then(|| ItemProps::from_map(&map))
}

/// Reads the menu layout, announcing the root menu first when `opening`.
async fn fetch_menu(conn: &Connection, owner: &str, path: &str, opening: bool) -> Option<MenuNode> {
    if opening {
        call(conn, owner, path, menu::INTERFACE, "AboutToShow", &0i32).await;
    }
    let reply = call(
        conn,
        owner,
        path,
        menu::INTERFACE,
        "GetLayout",
        &(0i32, -1i32, Vec::<String>::new()),
    )
    .await?;
    match reply.body().deserialize::<LayoutReply>() {
        Ok(layout) => Some(menu::parse_layout(&layout)),
        Err(err) => {
            tracing::debug!(%err, owner, path, "tray menu layout");
            None
        }
    }
}

/// Signals from `owner` on `path` (and `interface`, if given).
async fn signals(
    conn: &Connection,
    owner: &str,
    path: &str,
    interface: Option<&str>,
) -> Option<MessageStream> {
    let rule = || -> zbus::Result<MatchRule<'_>> {
        let mut builder = MatchRule::builder()
            .msg_type(message::Type::Signal)
            .sender(owner)?
            .path(path)?;
        if let Some(interface) = interface {
            builder = builder.interface(interface)?;
        }
        Ok(builder.build())
    };
    let stream = match rule() {
        Ok(rule) => MessageStream::for_match_rule(rule, conn, None).await,
        Err(err) => Err(err),
    };
    stream
        .inspect_err(|err| tracing::debug!(%err, owner, path, "tray signal subscription"))
        .ok()
}

/// Requests to an item's task.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Control {
    /// Read the menu layout again.
    RefreshMenu,
}

/// What woke the item task.
enum Wake {
    Item(Option<zbus::Result<Message>>),
    Menu(Option<zbus::Result<Message>>),
    Control(Option<Control>),
}

/// What an item signal asks for.
#[derive(Clone, Copy, Debug, Default)]
struct Refresh {
    props: bool,
    menu: bool,
}

impl Refresh {
    fn item_signal(&mut self, message: &Message) {
        let header = message.header();
        if header.member().is_some_and(|m| m.as_str() != "NewToolTip") {
            self.props = true;
        }
    }

    fn menu_signal(&mut self, message: &Message) {
        let header = message.header();
        if header
            .member()
            .is_some_and(|m| matches!(m.as_str(), "LayoutUpdated" | "ItemsPropertiesUpdated"))
        {
            self.menu = true;
        }
    }
}

/// Waits for the next message of an optional stream (forever without).
async fn next(stream: &mut Option<MessageStream>) -> Option<zbus::Result<Message>> {
    match stream {
        Some(stream) => stream.next().await,
        None => future::pending().await,
    }
}

/// Follows one item: reads its properties and menu, re-reads them on its
/// signals, and reports to the service loop until the item disappears or
/// the task is dropped.
pub async fn follow(
    conn: Connection,
    item: Registration,
    generation: u64,
    outbox: async_channel::Sender<service::Message>,
    control: async_channel::Receiver<Control>,
) {
    let key = item.key();
    let send = |message| {
        let _ = outbox.try_send(message);
    };
    let gone = || service::Message::Gone {
        key: key.clone(),
        generation,
    };
    let Some(owner) = owner(&conn, &item.service).await else {
        send(gone());
        return;
    };
    let mut item_signals = signals(&conn, &owner, &item.path, None).await;
    let mut props = None;
    for attempt in 0..INITIAL_ATTEMPTS {
        if attempt > 0 {
            Timer::after(RETRY_DELAY).await;
        }
        props = fetch_props(&conn, &owner, &item.path).await;
        if props.is_some() {
            break;
        }
    }
    let Some(mut props) = props else {
        tracing::debug!(key, "tray item has no readable properties");
        send(gone());
        return;
    };
    let mut menu_path: Option<String> = None;
    let mut menu_signals = None;
    loop {
        let mut menu_changed = false;
        if props.menu != menu_path {
            menu_path.clone_from(&props.menu);
            menu_signals = match &menu_path {
                Some(path) => signals(&conn, &owner, path, Some(menu::INTERFACE)).await,
                None => None,
            };
            menu_changed = true;
        }
        send(service::Message::Props {
            key: key.clone(),
            generation,
            props: Box::new(props.clone()),
        });
        let mut refresh = Refresh {
            props: false,
            menu: menu_changed,
        };
        loop {
            if refresh.menu {
                let menu = match &menu_path {
                    Some(path) => fetch_menu(&conn, &owner, path, menu_changed).await,
                    None => None,
                };
                menu_changed = false;
                send(service::Message::Menu {
                    key: key.clone(),
                    generation,
                    menu,
                });
                refresh.menu = false;
            }
            if refresh.props {
                break;
            }
            let wake = async { Wake::Item(next(&mut item_signals).await) }
                .or(async { Wake::Menu(next(&mut menu_signals).await) })
                .or(async { Wake::Control(control.recv().await.ok()) })
                .await;
            match wake {
                Wake::Item(Some(Ok(message))) => refresh.item_signal(&message),
                Wake::Menu(Some(Ok(message))) => refresh.menu_signal(&message),
                Wake::Control(Some(Control::RefreshMenu)) => refresh.menu = true,
                Wake::Item(None) | Wake::Control(None) => return,
                Wake::Menu(None) => menu_signals = None,
                Wake::Item(Some(Err(err))) | Wake::Menu(Some(Err(err))) => {
                    tracing::debug!(%err, key, "tray item signal");
                }
            }
            Timer::after(SIGNAL_DEBOUNCE).await;
            drain(&mut item_signals, |m| refresh.item_signal(m)).await;
            drain(&mut menu_signals, |m| refresh.menu_signal(m)).await;
        }
        if let Some(fresh) = fetch_props(&conn, &owner, &item.path).await {
            props = fresh;
        }
    }
}

/// Takes the messages already queued on `stream`.
async fn drain(stream: &mut Option<MessageStream>, mut each: impl FnMut(&Message)) {
    let Some(stream) = stream else {
        return;
    };
    while let Some(Some(message)) = future::poll_once(stream.next()).await {
        if let Ok(message) = message {
            each(&message);
        }
    }
}

#[cfg(test)]
mod tests {
    use zbus::zvariant::ObjectPath;

    use super::*;

    #[test]
    fn reads_item_properties() {
        let icons = Value::from(vec![(1i32, 1i32, vec![255u8, 1, 2, 3])]);
        let props: Vec<(&str, Value<'_>)> = vec![
            ("Id", Value::from("nm-applet")),
            ("Title", Value::from("")),
            ("Status", Value::from("NeedsAttention")),
            ("IconName", Value::from("network-wireless")),
            ("IconPixmap", icons),
            ("IconThemePath", Value::from(5i32)),
            ("ItemIsMenu", Value::from(true)),
            (
                "Menu",
                Value::from(ObjectPath::from_static_str("/MenuBar").expect("path")),
            ),
            ("Unknown", Value::from(1u8)),
        ];
        let item = ItemProps::from_props(props.iter().map(|(k, v)| (*k, v)));
        assert_eq!(
            item,
            ItemProps {
                id: Some("nm-applet".to_owned()),
                title: None,
                status: ItemStatus::NeedsAttention,
                icon_name: Some("network-wireless".to_owned()),
                icon_pixmaps: vec![RawPixmap {
                    width: 1,
                    height: 1,
                    argb: vec![255, 1, 2, 3],
                }],
                item_is_menu: true,
                menu: Some("/MenuBar".to_owned()),
                ..ItemProps::default()
            }
        );
        let no_menu = [(
            "Menu",
            Value::from(ObjectPath::from_static_str("/NO_DBUSMENU").expect("path")),
        )];
        assert_eq!(
            ItemProps::from_props(no_menu.iter().map(|(k, v)| (*k, v))).menu,
            None
        );
    }
}
