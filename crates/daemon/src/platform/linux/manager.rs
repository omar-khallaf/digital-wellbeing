use std::collections::HashMap;

use futures::StreamExt;

use tracing::{error, info, warn};

use crate::platform::{PlatformEvent, PowerEventKind};
use wellbeing_core::{
    PluginInstanceId, Uid,
    dbus_constants::{
        EVENT_FIELD_APP_ID, EVENT_FIELD_PID, EVENT_FIELD_POWER_TAG, EVENT_FIELD_TAG,
        EVENT_FIELD_TITLE, EVENT_POWER_HIBERNATE, EVENT_POWER_SHUTDOWN, EVENT_POWER_SUSPEND,
        EVENT_STRUCT_FIELD_COUNT, EVENT_TAG_BLOCK, EVENT_TAG_FOCUS, EVENT_TAG_IDLE,
        EVENT_TAG_LOCKED, EVENT_TAG_LOGOUT, EVENT_TAG_POWER, EVENT_TAG_RESUME, EVENT_TAG_UNFOCUS,
    },
};
use zbus::proxy;
use zvariant::OwnedValue;

// ── Proxy ────────────────────────────────────────────────────────────────────

/// The `org.wellbeing.v1.Manager` D-Bus interface exposed by the compositor plugin.
///
/// Signals:
///   - `Event(payload: OwnedValue)` — unified event carrying a `(ussuu)` struct.
#[proxy(
    interface = "org.wellbeing.v1.Manager",
    default_path = "/org/wellbeing/Manager"
)]
pub trait Manager {
    /// Unified event signal — `payload` is a D-Bus struct with signature `(ussuu)`:
    ///
    ///   field | type   | contents
    ///   ------+--------+-----------------------------------------------
    ///   0     | u32    | event tag (EVENT_TAG_FOCUS / _UNFOCUS / …)
    ///   1     | string | app_id (Focus, Block)
    ///   2     | string | title  (Focus, Block)
    ///   3     | u32    | pid    (Focus)
    ///   4     | u32    | power_tag  (PowerEvent: Suspend / Hibernate / Shutdown)
    #[zbus(signal)]
    fn event(&self, payload: OwnedValue) -> zbus::Result<()>;

    /// Read-only property returning the current focus state in the same
    /// `(uussuu)` struct format as the `Event` signal.  The client should
    /// use [`parse_event_payload`] to decode the value.
    #[zbus(property)]
    fn current_focus(&self) -> zbus::Result<OwnedValue>;
}

// ── Client ───────────────────────────────────────────────────────────────────

pub struct ManagerClient {
    pub uid: Uid,
    proxy: ManagerProxy<'static>,
}

impl ManagerClient {
    pub fn new(uid: Uid, proxy: ManagerProxy<'static>) -> Self {
        Self { uid, proxy }
    }

    /// Query the plugin's CurrentFocus D-Bus property.
    /// Returns the parsed [`PlatformEvent`] — including `Unfocus` when
    /// no app window is focused (the plugin sends [`EVENT_TAG_UNFOCUS`]
    /// with the uid).
    pub async fn current_focus(&self) -> Option<PlatformEvent> {
        let val: OwnedValue = self.proxy.current_focus().await.ok()?;
        parse_event_payload(val, self.uid)
    }
}

// ── Registry ─────────────────────────────────────────────────────────────────

pub struct PluginRegistry {
    clients: HashMap<PluginInstanceId, ManagerClient>,
    by_uid: HashMap<Uid, PluginInstanceId>,
}

impl PluginRegistry {
    pub fn new() -> Self {
        Self {
            clients: HashMap::new(),
            by_uid: HashMap::new(),
        }
    }

    pub fn register(
        &mut self,
        instance_id: PluginInstanceId,
        uid: Uid,
        proxy: ManagerProxy<'static>,
    ) {
        if let Some(old_id) = self.by_uid.insert(uid, instance_id.clone()) {
            self.clients.remove(&old_id);
            warn!(?uid, "replaced plugin instance");
        }
        let client = ManagerClient::new(uid, proxy);
        info!(?instance_id, ?uid, "plugin registered");
        self.clients.insert(instance_id, client);
    }

    pub fn unregister(&mut self, instance_id: &PluginInstanceId) {
        if let Some(client) = self.clients.remove(instance_id) {
            self.by_uid.remove(&client.uid);
            info!(?client.uid, "plugin unregistered");
        }
    }

    pub fn registered_uids(&self) -> Vec<Uid> {
        self.by_uid.keys().copied().collect()
    }

    /// Query the CurrentFocus property for the plugin registered to `uid`.
    /// Returns None if no plugin is registered for that uid, or if the
    /// property query fails.
    pub async fn current_focus_for_uid(&self, uid: Uid) -> Option<PlatformEvent> {
        let instance_id = self.by_uid.get(&uid)?;
        let client = self.clients.get(instance_id)?;
        client.current_focus().await
    }

    /// Subscribe to the unified `Event` signal from a plugin instance.
    /// Returns a receiver that yields parsed [`PlatformEvent`]s.
    pub async fn subscribe_signals(
        &self,
        instance_id: &PluginInstanceId,
        handle: &tokio::runtime::Handle,
    ) -> Option<tokio::sync::mpsc::Receiver<PlatformEvent>> {
        let client = self.clients.get(instance_id)?;
        let proxy = client.proxy.clone();
        let uid = client.uid;
        let (tx, rx) = tokio::sync::mpsc::channel::<PlatformEvent>(256);

        handle.spawn(async move {
            let mut event_stream = match proxy.receive_event().await {
                Ok(s) => s,
                Err(e) => {
                    error!("failed to subscribe Event signal: {e}");
                    return;
                }
            };

            loop {
                tokio::select! {
                    Some(signal) = event_stream.next() => {
                            if let Ok(args) = signal.args()
                                && let Some(event) = parse_event_payload(args.payload, uid) {
                                tx.send(event).await.ok();
                            }
                    }
                    else => break,
                }
            }
        });

        Some(rx)
    }
}

impl Default for PluginRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ── Payload parser (D-Bus → PlatformEvent boundary) ─────────────────────────

/// Parse a unified `Event` signal / `CurrentFocus` property payload into a
/// [`PlatformEvent`].
///
/// The payload is a D-Bus struct with signature `(ussuu)`:
///
///   [`EVENT_FIELD_TAG`]       = u32 tag  (EVENT_TAG_FOCUS / …)
///   [`EVENT_FIELD_APP_ID`]    = string app_id
///   [`EVENT_FIELD_TITLE`]     = string title
///   [`EVENT_FIELD_PID`]       = u32 pid
///   [`EVENT_FIELD_POWER_TAG`] = u32 power_tag
///
/// Returns `None` on malformed input (invalid tag, bad struct shape, or
/// empty `app_id` for Focus/Block which fails [`wellbeing_core::AppId`]
/// validation).
pub fn parse_event_payload(val: OwnedValue, uid: Uid) -> Option<PlatformEvent> {
    use zvariant::Value;

    // Every variant must be a struct with at least EVENT_STRUCT_FIELD_COUNT fields.
    let v: Value = val.into();
    let f = match &v {
        Value::Structure(s) if s.fields().len() >= EVENT_STRUCT_FIELD_COUNT => s.fields(),
        _ => return None,
    };

    match &f[EVENT_FIELD_TAG] {
        Value::U32(EVENT_TAG_FOCUS) => {
            let app_id_str = match &f[EVENT_FIELD_APP_ID] {
                Value::Str(s) => s.as_str(),
                _ => return None,
            };
            let title_str = match &f[EVENT_FIELD_TITLE] {
                Value::Str(s) => s.as_str(),
                _ => return None,
            };
            let pid_val = match &f[EVENT_FIELD_PID] {
                Value::U32(p) => wellbeing_core::Pid(*p),
                _ => return None,
            };
            let aid = wellbeing_core::AppId::new(app_id_str).ok()?;
            Some(PlatformEvent::Focus {
                app_id: aid,
                title: wellbeing_core::WindowTitle::new(title_str),
                pid: pid_val,
                uid,
            })
        }
        Value::U32(EVENT_TAG_UNFOCUS) => Some(PlatformEvent::Unfocus { uid }),
        Value::U32(EVENT_TAG_BLOCK) => {
            let app_id_str = match &f[EVENT_FIELD_APP_ID] {
                Value::Str(s) => s.as_str(),
                _ => return None,
            };
            let title_str = match &f[EVENT_FIELD_TITLE] {
                Value::Str(s) => s.as_str(),
                _ => return None,
            };
            let aid = wellbeing_core::AppId::new(app_id_str).ok()?;
            Some(PlatformEvent::Block {
                app_id: aid,
                title: wellbeing_core::WindowTitle::new(title_str),
                uid,
            })
        }
        Value::U32(EVENT_TAG_IDLE) => Some(PlatformEvent::Idle { uid }),
        Value::U32(EVENT_TAG_RESUME) => Some(PlatformEvent::Resume { uid }),
        Value::U32(EVENT_TAG_LOGOUT) => Some(PlatformEvent::LogOut { uid }),
        Value::U32(EVENT_TAG_LOCKED) => Some(PlatformEvent::Locked { uid }),
        Value::U32(EVENT_TAG_POWER) => {
            let kind = match &f[EVENT_FIELD_POWER_TAG] {
                Value::U32(EVENT_POWER_HIBERNATE) => PowerEventKind::Hibernate,
                Value::U32(EVENT_POWER_SHUTDOWN) => PowerEventKind::Shutdown,
                Value::U32(EVENT_POWER_SUSPEND) => PowerEventKind::Suspend,
                _ => return None,
            };
            Some(PlatformEvent::PowerEvent { kind, uid })
        }
        _ => None,
    }
}
