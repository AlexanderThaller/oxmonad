//! `org.kde.StatusNotifierWatcher`: served when no other watcher runs.

use std::sync::{
    Arc,
    Mutex,
    PoisonError,
};

use zbus::{
    interface,
    message::Header,
    names::UniqueName,
    object_server::SignalEmitter,
};

use super::service::Message;

/// The watcher's well-known bus name.
pub const NAME: &str = "org.kde.StatusNotifierWatcher";
/// The watcher's object path.
pub const PATH: &str = "/StatusNotifierWatcher";
/// The watcher's interface.
pub const INTERFACE: &str = "org.kde.StatusNotifierWatcher";
/// The item path when an item registers with just a bus name.
pub const DEFAULT_ITEM_PATH: &str = "/StatusNotifierItem";

/// A registered item: the bus name it lives on and its object path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Registration {
    pub service: String,
    pub path: String,
}

impl Registration {
    /// The item's key in the watcher's item list: service followed by
    /// path.
    pub fn key(&self) -> String {
        format!("{}{}", self.service, self.path)
    }
}

/// Parses an entry of a watcher's `RegisteredStatusNotifierItems` or a
/// `StatusNotifierItemRegistered` argument: `service/path` or just
/// `service`.
pub fn parse_entry(entry: &str) -> Option<Registration> {
    let (service, path) = match entry.find('/') {
        Some(slash) => entry.split_at(slash),
        None => (entry, DEFAULT_ITEM_PATH),
    };
    (!service.is_empty()).then(|| Registration {
        service: service.to_owned(),
        path: path.to_owned(),
    })
}

/// Parses a `RegisterStatusNotifierItem` argument: a bus name (the item
/// is at [`DEFAULT_ITEM_PATH`]) or an object path on the `sender`'s
/// connection.
pub fn parse_registration(argument: &str, sender: Option<&str>) -> Option<Registration> {
    if argument.starts_with('/') {
        Some(Registration {
            service: sender.filter(|s| !s.is_empty())?.to_owned(),
            path: argument.to_owned(),
        })
    } else {
        parse_entry(argument)
    }
}

/// The served watcher object. Registrations are forwarded to the tray's
/// service loop, which owns the item list.
#[derive(Debug)]
pub struct Watcher {
    pub messages: async_channel::Sender<Message>,
    pub items: Arc<Mutex<Vec<String>>>,
}

#[interface(name = "org.kde.StatusNotifierWatcher")]
impl Watcher {
    /// An item announces itself.
    #[expect(
        clippy::needless_pass_by_value,
        reason = "zbus hands the header over by value"
    )]
    fn register_status_notifier_item(&self, service: &str, #[zbus(header)] header: Header<'_>) {
        let sender = header.sender().map(UniqueName::as_str);
        if let Some(registration) = parse_registration(service, sender) {
            let _ = self.messages.try_send(Message::Register(registration));
        } else {
            tracing::debug!(service, "invalid tray item registration");
        }
    }

    /// A host announces itself (other hosts are fine; nothing to track).
    async fn register_status_notifier_host(
        &self,
        service: &str,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) {
        tracing::debug!(service, "tray host registered");
        if let Err(err) = Self::status_notifier_host_registered(&emitter).await {
            tracing::debug!(%err, "tray host registered signal");
        }
    }

    /// The registered items, as `service/path`.
    #[zbus(property)]
    fn registered_status_notifier_items(&self) -> Vec<String> {
        self.items
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Always true: oxmonad itself is a host.
    #[zbus(property)]
    #[expect(clippy::unused_self, reason = "zbus properties are methods")]
    fn is_status_notifier_host_registered(&self) -> bool {
        true
    }

    /// The spec's protocol version.
    #[zbus(property)]
    #[expect(clippy::unused_self, reason = "zbus properties are methods")]
    fn protocol_version(&self) -> i32 {
        0
    }

    #[zbus(signal)]
    pub async fn status_notifier_item_registered(
        emitter: &SignalEmitter<'_>,
        service: &str,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    pub async fn status_notifier_item_unregistered(
        emitter: &SignalEmitter<'_>,
        service: &str,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn status_notifier_host_registered(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn status_notifier_host_unregistered(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reg(service: &str, path: &str) -> Registration {
        Registration {
            service: service.to_owned(),
            path: path.to_owned(),
        }
    }

    #[test]
    fn parses_registrations() {
        assert_eq!(
            parse_registration("org.kde.StatusNotifierItem-12-1", Some(":1.5")),
            Some(reg(
                "org.kde.StatusNotifierItem-12-1",
                "/StatusNotifierItem"
            ))
        );
        assert_eq!(
            parse_registration("/org/ayatana/NotificationItem/nm", Some(":1.7")),
            Some(reg(":1.7", "/org/ayatana/NotificationItem/nm"))
        );
        assert_eq!(
            parse_registration(":1.9/StatusNotifierItem", Some(":1.9")),
            Some(reg(":1.9", "/StatusNotifierItem"))
        );
        assert_eq!(parse_registration("/path", None), None);
        assert_eq!(parse_registration("", Some(":1.2")), None);
    }

    #[test]
    fn parses_entries_and_keys() {
        let entry = parse_entry(":1.40/org/blueman/sni").expect("entry");
        assert_eq!(
            entry,
            Registration {
                service: ":1.40".to_owned(),
                path: "/org/blueman/sni".to_owned(),
            }
        );
        assert_eq!(entry.key(), ":1.40/org/blueman/sni");
        assert_eq!(
            parse_entry("org.freedesktop.network-manager"),
            Some(reg(
                "org.freedesktop.network-manager",
                "/StatusNotifierItem"
            ))
        );
        assert_eq!(parse_entry("/only/path"), None);
    }
}
