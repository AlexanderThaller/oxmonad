//! End-to-end tests against a private `dbus-daemon` (ignored by default:
//! they need `dbus-daemon` installed).
//!
//! Run with `cargo test -p oxmonad tray::tests -- --ignored`.

#![expect(
    clippy::unused_self,
    reason = "the fake D-Bus objects' methods must take self"
)]

use std::{
    collections::HashMap,
    io::{
        BufRead,
        BufReader,
    },
    process::{
        Child,
        Command,
        Stdio,
    },
    sync::{
        Arc,
        Mutex,
    },
    time::{
        Duration,
        Instant,
    },
};

use calloop::{
    EventLoop,
    channel::{
        Channel,
        Event,
    },
};
use zbus::{
    interface,
    zvariant::{
        ObjectPath,
        OwnedValue,
        Value,
    },
};

use super::*;

/// A `dbus-daemon` killed on drop.
struct Bus {
    daemon: Child,
    address: String,
}

impl Bus {
    fn start() -> Self {
        let dir = std::env::temp_dir();
        let mut daemon = Command::new("dbus-daemon")
            .args(["--session", "--nofork", "--print-address"])
            .arg(format!("--address=unix:dir={}", dir.display()))
            .stdout(Stdio::piped())
            .spawn()
            .expect("dbus-daemon");
        let mut address = String::new();
        BufReader::new(daemon.stdout.take().expect("stdout"))
            .read_line(&mut address)
            .expect("address");
        Self {
            daemon,
            address: address.trim().to_owned(),
        }
    }
}

impl Drop for Bus {
    fn drop(&mut self) {
        let _ = self.daemon.kill();
        let _ = self.daemon.wait();
    }
}

type Calls = Arc<Mutex<Vec<String>>>;

struct FakeItem {
    calls: Calls,
}

#[interface(name = "org.kde.StatusNotifierItem")]
impl FakeItem {
    fn activate(&self, x: i32, y: i32) {
        self.calls
            .lock()
            .expect("lock")
            .push(format!("Activate {x} {y}"));
    }

    #[zbus(property)]
    fn id(&self) -> String {
        "fake".to_owned()
    }

    #[zbus(property)]
    fn title(&self) -> String {
        "Fake item".to_owned()
    }

    #[zbus(property)]
    fn status(&self) -> String {
        "Active".to_owned()
    }

    #[zbus(property)]
    fn icon_pixmap(&self) -> Vec<(i32, i32, Vec<u8>)> {
        vec![(1, 1, vec![255, 0, 0, 255]), (2, 2, vec![128; 16])]
    }

    #[zbus(property)]
    fn menu(&self) -> ObjectPath<'static> {
        ObjectPath::from_static_str("/Menu").expect("path")
    }
}

struct FakeMenu {
    calls: Calls,
}

#[interface(name = "com.canonical.dbusmenu")]
impl FakeMenu {
    #[expect(
        clippy::needless_pass_by_value,
        reason = "zbus hands arguments over by value"
    )]
    fn get_layout(&self, parent: i32, depth: i32, properties: Vec<String>) -> menu::LayoutReply {
        let wanted = properties.len();
        self.calls
            .lock()
            .expect("lock")
            .push(format!("GetLayout {parent} {depth} {wanted}"));
        let child = menu::tests::layout_value(7, &[("label", Value::from("_Quit"))], vec![]);
        (
            1,
            (
                0,
                HashMap::new(),
                vec![OwnedValue::try_from(child).expect("owned")],
            ),
        )
    }

    fn about_to_show(&self, id: i32) -> bool {
        self.calls
            .lock()
            .expect("lock")
            .push(format!("AboutToShow {id}"));
        false
    }

    fn event(&self, id: i32, event: &str, data: Value<'_>, timestamp: u32) {
        let data = data.downcast::<i32>().unwrap_or(-1);
        self.calls
            .lock()
            .expect("lock")
            .push(format!("Event {id} {event} {data} {timestamp}"));
    }
}

/// Collects a tray's events.
struct Listener {
    event_loop: EventLoop<'static, Option<Vec<TrayItem>>>,
    latest: Option<Vec<TrayItem>>,
}

impl Listener {
    fn new(channel: Channel<TrayEvent>) -> Self {
        let event_loop = EventLoop::try_new().expect("event loop");
        event_loop
            .handle()
            .insert_source(channel, |event, (), latest: &mut Option<Vec<TrayItem>>| {
                if let Event::Msg(TrayEvent::Items(items)) = event {
                    *latest = Some(items);
                }
            })
            .expect("insert");
        Self {
            event_loop,
            latest: None,
        }
    }

    fn dispatch(&mut self) {
        self.event_loop
            .dispatch(Some(Duration::from_millis(50)), &mut self.latest)
            .expect("dispatch");
    }

    /// Dispatches until `done` accepts the latest item list.
    fn wait_for(&mut self, done: impl Fn(&[TrayItem]) -> bool) -> Vec<TrayItem> {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(items) = self.latest.as_ref().filter(|items| done(items)) {
                return items.clone();
            }
            assert!(
                Instant::now() < deadline,
                "timed out; last: {:?}",
                self.latest
            );
            self.dispatch();
        }
    }
}

fn wait_for_call(calls: &Calls, call: &str) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !calls.lock().expect("lock").iter().any(|c| c == call) {
        assert!(Instant::now() < deadline, "no {call}: {calls:?}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Connects a fake item (with a menu at `/Menu` if `with_menu`) and
/// registers it by object path.
fn fake_item(bus: &Bus, calls: &Calls, with_menu: bool) -> zbus::Connection {
    zbus::block_on(async {
        let mut builder = zbus::connection::Builder::address(bus.address.as_str())?.serve_at(
            "/StatusNotifierItem",
            FakeItem {
                calls: Arc::clone(calls),
            },
        )?;
        if with_menu {
            builder = builder.serve_at(
                "/Menu",
                FakeMenu {
                    calls: Arc::clone(calls),
                },
            )?;
        }
        let conn = builder.build().await?;
        conn.call_method(
            Some("org.kde.StatusNotifierWatcher"),
            "/StatusNotifierWatcher",
            Some("org.kde.StatusNotifierWatcher"),
            "RegisterStatusNotifierItem",
            &"/StatusNotifierItem",
        )
        .await?;
        zbus::Result::Ok(conn)
    })
    .expect("fake item")
}

/// The watcher's `RegisteredStatusNotifierItems`.
async fn registered_items(conn: &zbus::Connection) -> zbus::Result<Vec<String>> {
    let reply = conn
        .call_method(
            Some("org.kde.StatusNotifierWatcher"),
            "/StatusNotifierWatcher",
            Some("org.freedesktop.DBus.Properties"),
            "Get",
            &(
                "org.kde.StatusNotifierWatcher",
                "RegisteredStatusNotifierItems",
            ),
        )
        .await?;
    let value: OwnedValue = reply.body().deserialize()?;
    Ok(Vec::<String>::try_from(value)?)
}

/// A menu entry with defaults.
fn entry(id: i32, label: &str, children: Vec<MenuNode>) -> MenuNode {
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

#[test]
#[ignore = "needs dbus-daemon"]
fn watcher_follows_a_fake_item() {
    let bus = Bus::start();
    let (tray, channel) = Tray::start_on(11, Some(bus.address.clone())).expect("tray");
    let mut listener = Listener::new(channel);
    listener.wait_for(<[TrayItem]>::is_empty);

    let calls = Calls::default();
    let item = fake_item(&bus, &calls, true);
    let id = format!("{}/StatusNotifierItem", item.unique_name().expect("unique"));
    let items = listener.wait_for(|items| items.first().is_some_and(|i| i.menu.is_some()));
    assert_eq!(
        items,
        vec![TrayItem {
            id: id.clone(),
            title: "Fake item".to_owned(),
            status: ItemStatus::Active,
            // 22 pixels wanted: the largest pixmap, premultiplied BGRA.
            icon: Some(TrayIcon {
                width: 2,
                height: 2,
                bgra: [64, 64, 64, 128].repeat(4),
            }),
            item_is_menu: false,
            menu: Some(entry(0, "", vec![entry(7, "Quit", vec![])])),
        }]
    );
    wait_for_call(&calls, "AboutToShow 0");
    wait_for_call(&calls, "GetLayout 0 -1 0");

    tray.activate(&id, 10, 20);
    wait_for_call(&calls, "Activate 10 20");
    tray.menu_about_to_show(&id, 7);
    wait_for_call(&calls, "AboutToShow 7");
    tray.menu_clicked(&id, 7);
    wait_for_call(&calls, "Event 7 clicked 0 0");

    // The registered list is visible to other hosts.
    let registered = zbus::block_on(registered_items(&item)).expect("registered items");
    assert_eq!(registered, vec![id]);

    // The item's connection closing removes it.
    drop(item);
    listener.wait_for(<[TrayItem]>::is_empty);
    let registered = zbus::block_on(async {
        let conn = zbus::connection::Builder::address(bus.address.as_str())?
            .build()
            .await?;
        registered_items(&conn).await
    })
    .expect("registered items");
    assert!(registered.is_empty(), "{registered:?}");
}

#[test]
#[ignore = "needs dbus-daemon"]
fn host_follows_another_watcher_and_takes_over() {
    let bus = Bus::start();
    let (watcher, watcher_channel) = Tray::start_on(11, Some(bus.address.clone())).expect("tray");
    let mut watcher_listener = Listener::new(watcher_channel);
    watcher_listener.wait_for(<[TrayItem]>::is_empty);

    // A second tray finds the watcher taken and acts as a host.
    let (_host, host_channel) = Tray::start_on(11, Some(bus.address.clone())).expect("host");
    let mut listener = Listener::new(host_channel);
    listener.wait_for(<[TrayItem]>::is_empty);
    let calls = Calls::default();
    let item = fake_item(&bus, &calls, false);
    let seen = listener.wait_for(|items| items.len() == 1);
    assert_eq!(seen[0].title, "Fake item");
    assert_eq!(seen[0].menu, None);

    // When the watcher goes away, the host takes over and keeps the items
    // it knew.
    drop(watcher);
    drop(watcher_listener);
    let key = format!("{}/StatusNotifierItem", item.unique_name().expect("unique"));
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let registered = zbus::block_on(registered_items(&item)).unwrap_or_default();
        if registered == vec![key.clone()] {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "host did not take over: {registered:?}"
        );
        listener.dispatch();
    }
    assert_eq!(listener.latest.map(|items| items.len()), Some(1));
}
