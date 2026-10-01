//! The tray thread: owns the D-Bus connection, the item list and the
//! (optional) watcher role, and reports to the main loop.
//!
//! Everything runs on one local executor: this service loop, one
//! [`item::follow`] task per item and short-lived tasks for commands. The
//! loop itself only awaits its own bus requests; calls to items happen in
//! the tasks, each bounded by [`item::CALL_TIMEOUT`], so a hung client
//! never stalls the others.

use std::{
    io,
    path::Path,
    rc::Rc,
    sync::{
        Arc,
        Mutex,
        PoisonError,
        mpsc,
    },
    time::{
        Duration,
        Instant,
    },
};

use async_executor::{
    LocalExecutor,
    Task,
};
use async_io::Timer;
use futures_lite::{
    FutureExt,
    StreamExt,
};
use zbus::{
    Connection,
    MatchRule,
    MessageStream,
    connection,
    fdo::{
        DBusProxy,
        RequestNameFlags,
        RequestNameReply,
    },
    message,
    names::MemberName,
    zvariant::{
        OwnedValue,
        Value,
    },
};

use super::{
    Command,
    ItemStatus,
    MenuNode,
    TrayEvent,
    TrayIcon,
    TrayItem,
    icon::{
        IconCache,
        RawPixmap,
        pixmap_icon,
    },
    item::{
        self,
        Control,
        ItemProps,
        call,
        timeout,
    },
    menu,
    theme::IconThemes,
    watcher::{
        self,
        Registration,
        Watcher,
    },
};

/// Changes are collected this long before the main loop hears of them.
const DEBOUNCE: Duration = Duration::from_millis(50);

/// What the service loop handles.
#[derive(Debug)]
pub enum Message {
    /// From the main loop.
    Command(Command),
    /// An item registered with our watcher.
    Register(Registration),
    /// Another watcher's `StatusNotifierItem(Un)Registered` signal.
    WatcherSignal { registered: bool, entry: String },
    /// Another watcher's `RegisteredStatusNotifierItems`.
    WatcherItems(Vec<String>),
    /// `NameOwnerChanged`; `owner` is `None` when the name vanished.
    NameOwnerChanged { name: String, owner: Option<String> },
    /// An item's (re-read) properties.
    Props {
        key: String,
        generation: u64,
        props: Box<ItemProps>,
    },
    /// An item's (re-read) menu.
    Menu {
        key: String,
        generation: u64,
        menu: Option<MenuNode>,
    },
    /// An item turned out not to exist.
    Gone { key: String, generation: u64 },
}

/// What the tray thread starts with.
#[derive(Debug)]
pub struct Config {
    /// The bus address; the session bus when `None`.
    pub address: Option<String>,
    pub icon_size: u32,
    pub inbox: async_channel::Receiver<Message>,
    pub outbox: async_channel::Sender<Message>,
    pub events: calloop::channel::Sender<TrayEvent>,
}

/// Whether we serve the watcher or follow someone else's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Role {
    Watcher,
    Host,
}

/// One followed item.
#[derive(Debug)]
struct Entry {
    registration: Registration,
    key: String,
    generation: u64,
    menu_path: Option<String>,
    item: Option<TrayItem>,
    menu: Option<MenuNode>,
    control: async_channel::Sender<Control>,
    _task: Task<()>,
}

/// The tray thread's main function: connects, reports the outcome on
/// `ready`, then serves until the [`Tray`](super::Tray) is dropped.
pub fn run(config: Config, ready: &mpsc::SyncSender<io::Result<()>>) {
    let executor = Rc::new(LocalExecutor::new());
    let spawner = Rc::clone(&executor);
    async_io::block_on(executor.run(async move {
        let conn = match connect(config.address.as_deref()).await {
            Ok(conn) => conn,
            Err(err) => {
                let _ = ready.send(Err(io::Error::other(format!(
                    "cannot connect to the session bus: {err}"
                ))));
                return;
            }
        };
        if ready.send(Ok(())).is_err() {
            return;
        }
        let service = Service::new(conn, spawner, config);
        if let Err(err) = service.serve().await {
            tracing::warn!(%err, "system tray stopped");
        }
    }));
}

async fn connect(address: Option<&str>) -> zbus::Result<Connection> {
    let builder = match address {
        Some(address) => connection::Builder::address(address)?,
        None => connection::Builder::session()?,
    };
    builder.build().await
}

/// The best icon from pixmaps or a name.
fn resolve_icon(
    icons: &mut IconCache,
    pixmaps: &[RawPixmap],
    name: Option<&str>,
    theme_path: Option<&Path>,
    size: u32,
) -> Option<TrayIcon> {
    pixmap_icon(pixmaps, size).or_else(|| icons.named(name?, theme_path, size))
}

/// A list of strings, as in `RegisteredStatusNotifierItems`.
fn strings(value: &Value<'_>) -> Vec<String> {
    let Value::Array(array) = menu::unwrap_variant(value) else {
        return Vec::new();
    };
    array
        .inner()
        .iter()
        .filter_map(|v| {
            if let Value::Str(s) = v {
                Some(s.to_string())
            } else {
                None
            }
        })
        .collect()
}

struct Service {
    conn: Connection,
    executor: Rc<LocalExecutor<'static>>,
    inbox: async_channel::Receiver<Message>,
    outbox: async_channel::Sender<Message>,
    events: calloop::channel::Sender<TrayEvent>,
    /// Pixel size icons are prepared for.
    pixels: u32,
    icons: IconCache,
    role: Role,
    host_name: String,
    /// The served watcher's `RegisteredStatusNotifierItems`.
    registered: Arc<Mutex<Vec<String>>>,
    entries: Vec<Entry>,
    next_generation: u64,
    /// When pending changes are reported.
    flush_at: Option<Instant>,
    sent: Option<Vec<TrayItem>>,
    /// Background tasks living as long as the service.
    tasks: Vec<Task<()>>,
}

impl Service {
    fn new(conn: Connection, executor: Rc<LocalExecutor<'static>>, config: Config) -> Self {
        Self {
            conn,
            executor,
            inbox: config.inbox,
            outbox: config.outbox,
            events: config.events,
            pixels: config.icon_size.saturating_mul(2),
            icons: IconCache::new(IconThemes::from_env()),
            role: Role::Host,
            host_name: format!("org.kde.StatusNotifierHost-{}", std::process::id()),
            registered: Arc::new(Mutex::new(Vec::new())),
            entries: Vec::new(),
            next_generation: 0,
            flush_at: None,
            sent: None,
            tasks: Vec::new(),
        }
    }

    /// Sets up the bus side, then handles messages until the tray handle
    /// is dropped or the main loop is gone.
    async fn serve(mut self) -> zbus::Result<()> {
        self.conn
            .object_server()
            .at(
                watcher::PATH,
                Watcher {
                    messages: self.outbox.clone(),
                    items: Arc::clone(&self.registered),
                },
            )
            .await?;
        self.forward_name_owner_changes().await?;
        self.forward_watcher_signals().await?;
        if let Err(err) = self
            .conn
            .request_name_with_flags(self.host_name.as_str(), RequestNameFlags::DoNotQueue.into())
            .await
        {
            tracing::debug!(%err, "cannot own the tray host name");
        }
        if self.claim_watcher().await {
            tracing::debug!("serving the StatusNotifierWatcher");
        } else {
            tracing::debug!("another StatusNotifierWatcher runs; registering as a host");
            self.register_host();
        }
        // Report the (empty) list once so the bar knows the tray works.
        self.flush_at = Some(Instant::now());
        loop {
            let inbox = self.inbox.clone();
            let message = match self.flush_at {
                Some(at) => {
                    async { Some(inbox.recv().await) }
                        .or(async {
                            Timer::at(at).await;
                            None
                        })
                        .await
                }
                None => Some(inbox.recv().await),
            };
            match message {
                None => {
                    self.flush_at = None;
                    if !self.flush() {
                        return Ok(());
                    }
                }
                Some(Ok(message)) => self.handle(message).await,
                Some(Err(_)) => return Ok(()),
            }
        }
    }

    fn spawn(&self, future: impl Future<Output = ()> + 'static) {
        self.executor.spawn(future).detach();
    }

    async fn forward_name_owner_changes(&mut self) -> zbus::Result<()> {
        let mut changes = DBusProxy::new(&self.conn)
            .await?
            .receive_name_owner_changed()
            .await?;
        let outbox = self.outbox.clone();
        self.tasks.push(self.executor.spawn(async move {
            while let Some(change) = changes.next().await {
                let Ok(args) = change.args() else {
                    continue;
                };
                let message = Message::NameOwnerChanged {
                    name: args.name().to_string(),
                    owner: args.new_owner().as_ref().map(ToString::to_string),
                };
                if outbox.send(message).await.is_err() {
                    break;
                }
            }
        }));
        Ok(())
    }

    async fn forward_watcher_signals(&mut self) -> zbus::Result<()> {
        let rule = MatchRule::builder()
            .msg_type(message::Type::Signal)
            .interface(watcher::INTERFACE)?
            .path(watcher::PATH)?
            .build();
        let mut signals = MessageStream::for_match_rule(rule, &self.conn, None).await?;
        let outbox = self.outbox.clone();
        self.tasks.push(self.executor.spawn(async move {
            while let Some(signal) = signals.next().await {
                let Ok(signal) = signal else {
                    continue;
                };
                let header = signal.header();
                let registered = match header.member().map(MemberName::as_str) {
                    Some("StatusNotifierItemRegistered") => true,
                    Some("StatusNotifierItemUnregistered") => false,
                    _ => continue,
                };
                let Ok(entry) = signal.body().deserialize::<String>() else {
                    continue;
                };
                if outbox
                    .send(Message::WatcherSignal { registered, entry })
                    .await
                    .is_err()
                {
                    break;
                }
            }
        }));
        Ok(())
    }

    /// Tries to own the watcher name without replacing anyone.
    async fn claim_watcher(&mut self) -> bool {
        let request = self
            .conn
            .request_name_with_flags(watcher::NAME, RequestNameFlags::DoNotQueue.into());
        let claimed = matches!(
            timeout(item::CALL_TIMEOUT, request).await,
            Some(Ok(
                RequestNameReply::PrimaryOwner | RequestNameReply::AlreadyOwner
            ))
        );
        if claimed {
            self.role = Role::Watcher;
            self.sync_registered().await;
        }
        claimed
    }

    /// Registers as a host with the current watcher and reads its items.
    fn register_host(&self) {
        let conn = self.conn.clone();
        let host = self.host_name.clone();
        let outbox = self.outbox.clone();
        self.spawn(async move {
            call(
                &conn,
                watcher::NAME,
                watcher::PATH,
                watcher::INTERFACE,
                "RegisterStatusNotifierHost",
                &host,
            )
            .await;
            let reply = call(
                &conn,
                watcher::NAME,
                watcher::PATH,
                "org.freedesktop.DBus.Properties",
                "Get",
                &(watcher::INTERFACE, "RegisteredStatusNotifierItems"),
            )
            .await;
            if let Some(reply) = reply
                && let Ok(value) = reply.body().deserialize::<OwnedValue>()
            {
                let _ = outbox.send(Message::WatcherItems(strings(&value))).await;
            }
        });
    }

    async fn handle(&mut self, message: Message) {
        match message {
            Message::Command(command) => self.command(command),
            Message::Register(registration) if self.role == Role::Watcher => {
                let key = registration.key();
                self.add(registration);
                self.sync_registered().await;
                if let Some(watcher) = self.watcher_ref().await
                    && let Err(err) =
                        Watcher::status_notifier_item_registered(watcher.signal_emitter(), &key)
                            .await
                {
                    tracing::debug!(%err, "tray item registered signal");
                }
            }
            Message::WatcherSignal { registered, entry } if self.role == Role::Host => {
                let Some(registration) = watcher::parse_entry(&entry) else {
                    return;
                };
                if registered {
                    self.add(registration);
                } else {
                    self.remove(|e| e.registration == registration).await;
                }
            }
            Message::WatcherItems(entries) if self.role == Role::Host => {
                let registrations: Vec<Registration> = entries
                    .iter()
                    .filter_map(|e| watcher::parse_entry(e))
                    .collect();
                self.remove(|e| !registrations.contains(&e.registration))
                    .await;
                for registration in registrations {
                    self.add(registration);
                }
            }
            Message::NameOwnerChanged { name, owner } => {
                self.name_owner_changed(&name, owner).await;
            }
            Message::Props {
                key,
                generation,
                props,
            } => self.update_props(&key, generation, *props),
            Message::Menu {
                key,
                generation,
                menu,
            } => {
                if let Some(entry) = self.entry_mut(&key, generation) {
                    entry.menu = menu;
                    self.changed();
                }
            }
            Message::Gone { key, generation } => {
                self.remove(|e| e.key == key && e.generation == generation)
                    .await;
            }
            Message::Register(_) | Message::WatcherSignal { .. } | Message::WatcherItems(_) => {}
        }
    }

    /// Stores an item's new properties, resolving its icon.
    fn update_props(&mut self, key: &str, generation: u64, props: ItemProps) {
        let pixels = self.pixels;
        let Some(entry) = self.entry_mut(key, generation) else {
            return;
        };
        entry.menu_path.clone_from(&props.menu);
        let theme_path = props.theme_path.as_deref().map(Path::new);
        let normal = |icons: &mut IconCache| {
            resolve_icon(
                icons,
                &props.icon_pixmaps,
                props.icon_name.as_deref(),
                theme_path,
                pixels,
            )
        };
        let icons = &mut self.icons;
        let icon = if props.status == ItemStatus::NeedsAttention {
            resolve_icon(
                icons,
                &props.attention_pixmaps,
                props.attention_icon_name.as_deref(),
                theme_path,
                pixels,
            )
            .or_else(|| normal(icons))
        } else {
            normal(icons)
        };
        let item = TrayItem {
            id: key.to_owned(),
            title: props.title.or(props.id).unwrap_or_default(),
            status: props.status,
            icon,
            item_is_menu: props.item_is_menu,
            menu: None,
        };
        if let Some(entry) = self.entry_mut(key, generation) {
            entry.item = Some(item);
        }
        self.changed();
    }

    async fn name_owner_changed(&mut self, name: &str, owner: Option<String>) {
        if name == watcher::NAME {
            let ours = self.conn.unique_name().map(ToString::to_string);
            match owner {
                None if self.role == Role::Host => {
                    if self.claim_watcher().await {
                        tracing::debug!("took over the StatusNotifierWatcher");
                    }
                }
                Some(owner) if Some(&owner) != ours.as_ref() => {
                    // Someone else serves the watcher (again): follow it.
                    self.role = Role::Host;
                    self.register_host();
                }
                _ => {}
            }
        } else if owner.is_none() {
            self.remove(|e| e.registration.service == name).await;
        }
    }

    fn entry_mut(&mut self, key: &str, generation: u64) -> Option<&mut Entry> {
        self.entries
            .iter_mut()
            .find(|e| e.key == key && e.generation == generation)
    }

    /// Starts following an item, unless it is followed already.
    fn add(&mut self, registration: Registration) {
        let key = registration.key();
        if self.entries.iter().any(|e| e.key == key) {
            return;
        }
        tracing::debug!(key, "tray item added");
        let generation = self.next_generation;
        self.next_generation += 1;
        let (control, control_inbox) = async_channel::unbounded();
        let task = self.executor.spawn(item::follow(
            self.conn.clone(),
            registration.clone(),
            generation,
            self.outbox.clone(),
            control_inbox,
        ));
        self.entries.push(Entry {
            registration,
            key,
            generation,
            menu_path: None,
            item: None,
            menu: None,
            control,
            _task: task,
        });
    }

    /// Stops following the items matching `which`.
    async fn remove(&mut self, which: impl Fn(&Entry) -> bool) {
        let (removed, kept): (Vec<Entry>, Vec<Entry>) = std::mem::take(&mut self.entries)
            .into_iter()
            .partition(which);
        self.entries = kept;
        if removed.is_empty() {
            return;
        }
        self.changed();
        if self.role != Role::Watcher {
            return;
        }
        self.sync_registered().await;
        let Some(watcher) = self.watcher_ref().await else {
            return;
        };
        for entry in removed {
            tracing::debug!(key = entry.key, "tray item removed");
            if let Err(err) =
                Watcher::status_notifier_item_unregistered(watcher.signal_emitter(), &entry.key)
                    .await
            {
                tracing::debug!(%err, "tray item unregistered signal");
            }
        }
    }

    async fn watcher_ref(&self) -> Option<zbus::object_server::InterfaceRef<Watcher>> {
        self.conn
            .object_server()
            .interface::<_, Watcher>(watcher::PATH)
            .await
            .ok()
    }

    /// Publishes the item list as our watcher's property.
    async fn sync_registered(&self) {
        let keys: Vec<String> = self.entries.iter().map(|e| e.key.clone()).collect();
        *self
            .registered
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = keys;
        if let Some(watcher) = self.watcher_ref().await
            && let Err(err) = watcher
                .get()
                .await
                .registered_status_notifier_items_changed(watcher.signal_emitter())
                .await
        {
            tracing::debug!(%err, "tray items property change");
        }
    }

    /// Runs a command from the main loop against an item.
    fn command(&self, command: Command) {
        let (key, method) = match &command {
            Command::Activate { item, .. } => (item, "Activate"),
            Command::SecondaryActivate { item, .. } => (item, "SecondaryActivate"),
            Command::ContextMenu { item, .. } => (item, "ContextMenu"),
            Command::Scroll { item, .. } => (item, "Scroll"),
            Command::MenuAboutToShow { item, .. } => (item, "AboutToShow"),
            Command::MenuClicked { item, .. } => (item, "Event"),
        };
        let Some(entry) = self.entries.iter().find(|e| &e.key == key) else {
            tracing::debug!(key, "tray command for an unknown item");
            return;
        };
        let conn = self.conn.clone();
        let service = entry.registration.service.clone();
        let path = entry.registration.path.clone();
        let menu_path = entry.menu_path.clone();
        let control = entry.control.clone();
        self.spawn(async move {
            match command {
                Command::Activate { x, y, .. }
                | Command::SecondaryActivate { x, y, .. }
                | Command::ContextMenu { x, y, .. } => {
                    call(&conn, &service, &path, item::INTERFACE, method, &(x, y)).await;
                }
                Command::Scroll {
                    delta, horizontal, ..
                } => {
                    let orientation = if horizontal { "horizontal" } else { "vertical" };
                    call(
                        &conn,
                        &service,
                        &path,
                        item::INTERFACE,
                        method,
                        &(delta, orientation),
                    )
                    .await;
                }
                Command::MenuAboutToShow { id, .. } => {
                    let Some(menu_path) = menu_path else {
                        return;
                    };
                    let reply =
                        call(&conn, &service, &menu_path, menu::INTERFACE, method, &id).await;
                    if reply.is_some_and(|r| r.body().deserialize::<bool>().unwrap_or(false)) {
                        let _ = control.send(Control::RefreshMenu).await;
                    }
                }
                Command::MenuClicked { id, .. } => {
                    let Some(menu_path) = menu_path else {
                        return;
                    };
                    let body = (id, "clicked", Value::from(0i32), 0u32);
                    call(&conn, &service, &menu_path, menu::INTERFACE, method, &body).await;
                }
            }
        });
    }

    /// Schedules a report to the main loop.
    fn changed(&mut self) {
        if self.flush_at.is_none() {
            self.flush_at = Some(Instant::now() + DEBOUNCE);
        }
    }

    /// Reports the item list if it changed; `false` once the main loop is
    /// gone.
    fn flush(&mut self) -> bool {
        let items: Vec<TrayItem> = self
            .entries
            .iter()
            .filter_map(|entry| {
                let mut item = entry.item.clone()?;
                item.menu.clone_from(&entry.menu);
                Some(item)
            })
            .collect();
        if self.sent.as_ref() == Some(&items) {
            return true;
        }
        self.sent = Some(items.clone());
        self.events.send(TrayEvent::Items(items)).is_ok()
    }
}

impl std::fmt::Debug for Service {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Service")
            .field("role", &self.role)
            .field("entries", &self.entries.len())
            .finish_non_exhaustive()
    }
}
