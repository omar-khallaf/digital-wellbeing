use std::collections::HashMap;

use futures::StreamExt;

use tracing::{error, info, warn};

use crate::platform::{PlatformEvent, PowerEventKind};
use wellbeing_core::{
    PluginInstanceId, Uid,
    dbus_constants::{
        EVENT_FIELD_APP_ID, EVENT_FIELD_POWER_TAG, EVENT_FIELD_TAG, EVENT_FIELD_TITLE,
        EVENT_POWER_HIBERNATE, EVENT_POWER_SHUTDOWN, EVENT_POWER_SUSPEND, EVENT_STRUCT_FIELD_COUNT,
        EVENT_TAG_BLOCK, EVENT_TAG_FOCUS, EVENT_TAG_IDLE, EVENT_TAG_LOCKED, EVENT_TAG_LOGOUT,
        EVENT_TAG_POWER, EVENT_TAG_RESUME, EVENT_TAG_UNFOCUS,
    },
};
use zbus::proxy;
use zvariant::OwnedValue;

/// The `org.wellbeing.v1.Manager` D-Bus interface exposed by the compositor plugin.
///
/// Signals:
///   - `Event(payload: OwnedValue)` — unified event carrying a `(ussu)` struct.
#[proxy(
    interface = "org.wellbeing.v1.Manager",
    default_path = "/org/wellbeing/Manager"
)]
pub trait Manager {
    /// Unified event signal — `payload` is a D-Bus struct with signature `(ussu)`:
    ///
    ///   field | type   | contents
    ///   ------+--------+-----------------------------------------------
    ///   0     | u32    | event tag (EVENT_TAG_FOCUS / _UNFOCUS / …)
    ///   1     | string | app_class (Focus, Block)
    ///   2     | string | title  (Focus, Block)
    ///   3     | u32    | power_tag  (PowerEvent: Suspend / Hibernate / Shutdown)
    #[zbus(signal)]
    fn event(&self, payload: OwnedValue) -> zbus::Result<()>;

    /// Method returning the current focus state in the same `(ussu)` struct
    /// format as the `Event` signal.  The client should use
    /// [`parse_event_payload`] to decode the value.
    fn get_focus_state(&self) -> zbus::Result<OwnedValue>;
}

pub struct ManagerClient {
    pub uid: Uid,
    proxy: ManagerProxy<'static>,
}

impl ManagerClient {
    pub fn new(uid: Uid, proxy: ManagerProxy<'static>) -> Self {
        Self { uid, proxy }
    }

    /// Query the plugin's current focus state via `GetFocusState` D-Bus method.
    /// Returns the parsed [`PlatformEvent`] — including `Unfocus` when
    /// no app window is focused (the plugin sends [`EVENT_TAG_UNFOCUS`]
    /// with the uid).
    pub async fn current_focus(&self) -> Option<PlatformEvent> {
        let val = self.proxy.get_focus_state().await;
        match val {
            Ok(v) => {
                let parsed = parse_event_payload(v, self.uid);
                if parsed.is_none() {
                    warn!(uid = ?self.uid, "ManagerClient::current_focus: D-Bus call succeeded but parse failed");
                }
                parsed
            }
            Err(e) => {
                warn!(uid = ?self.uid, error = %e, "ManagerClient::current_focus: D-Bus call failed");
                None
            }
        }
    }
}

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

    /// Return all currently registered UIDs.
    pub fn registered_uids(&self) -> Vec<Uid> {
        self.by_uid.keys().copied().collect()
    }

    /// Clone the [`ManagerProxy`] for a registered uid without holding the
    /// registry lock across the subsequent D-Bus call.
    ///
    /// Returns `None` if no plugin is registered for that uid.
    pub fn focus_proxy_for_uid(&self, uid: Uid) -> Option<(Uid, ManagerProxy<'static>)> {
        let instance_id = self.by_uid.get(&uid)?;
        let client = self.clients.get(instance_id)?;
        Some((client.uid, client.proxy.clone()))
    }

    /// Remove a plugin registration by uid. Safe to call after detecting
    /// that the plugin's D-Bus connection is dead (e.g. `current_focus()`
    /// returned an error).
    pub fn unregister_by_uid(&mut self, uid: Uid) {
        if let Some(instance_id) = self.by_uid.remove(&uid) {
            self.clients.remove(&instance_id);
            info!(?uid, "plugin unregistered (stale/disconnected)");
        }
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
                        match signal.args() {
                            Ok(args) => {
                                match parse_event_payload(args.payload, uid) {
                                    Some(event) => {
                                        tx.send(event).await.ok();
                                    }
                                    None => {
                                        warn!(?uid, "subscribe_signals: failed to parse event payload from plugin");
                                    }
                                }
                            }
                            Err(e) => {
                                warn!(?uid, error = %e, "subscribe_signals: failed to deserialize signal args");
                            }
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

/// Parse a unified `Event` signal / `GetFocusState` reply payload into a
/// [`PlatformEvent`].
///
/// The payload is a D-Bus struct with signature `(ussu)`:
///
///   [`EVENT_FIELD_TAG`]       = u32 tag  (EVENT_TAG_FOCUS / …)
///   [`EVENT_FIELD_APP_ID`]    = string app_class
///   [`EVENT_FIELD_TITLE`]     = string title
///   [`EVENT_FIELD_POWER_TAG`] = u32 power_tag
///
/// Returns `None` on malformed input (invalid tag, bad struct shape, or
/// empty `app_class` for Focus/Block which fails [`wellbeing_core::AppClass`]
/// validation).
pub fn parse_event_payload(val: OwnedValue, uid: Uid) -> Option<PlatformEvent> {
    use zvariant::Value;

    let v: Value = val.into();
    // Unwrap outer Variant wrapper (D-Bus VARIANT / sdbus::Variant wrapping).
    let v = match v {
        Value::Value(boxed) => *boxed,
        other => other,
    };
    let f = match &v {
        Value::Structure(s) if s.fields().len() >= EVENT_STRUCT_FIELD_COUNT => s.fields(),
        other => {
            warn!(
                ?uid,
                value_type = ?std::mem::discriminant(other),
                value_debug = ?other,
                "parse_event_payload: expected struct — got unexpected Value shape"
            );
            return None;
        }
    };

    match &f[EVENT_FIELD_TAG] {
        Value::U32(EVENT_TAG_FOCUS) => {
            let app_class_str = match &f[EVENT_FIELD_APP_ID] {
                Value::Str(s) => s.as_str(),
                other => {
                    warn!(?uid, tag = "Focus", field = "app_class", value_debug = ?other, "parse_event_payload: expected string for app_class");
                    return None;
                }
            };
            let title_str = match &f[EVENT_FIELD_TITLE] {
                Value::Str(s) => s.as_str(),
                other => {
                    warn!(?uid, tag = "Focus", field = "title", value_debug = ?other, "parse_event_payload: expected string for title");
                    return None;
                }
            };
            let aid = match wellbeing_core::AppClass::new(app_class_str) {
                Ok(a) => a,
                Err(e) => {
                    warn!(?uid, tag = "Focus", app_class = app_class_str, error = %e, "parse_event_payload: AppClass validation failed");
                    return None;
                }
            };
            Some(PlatformEvent::Focus {
                app_class: aid,
                title: wellbeing_core::WindowTitle::new(title_str),
                uid,
            })
        }
        Value::U32(EVENT_TAG_UNFOCUS) => Some(PlatformEvent::Unfocus { uid }),
        Value::U32(EVENT_TAG_BLOCK) => {
            let app_class_str = match &f[EVENT_FIELD_APP_ID] {
                Value::Str(s) => s.as_str(),
                other => {
                    warn!(?uid, tag = "Block", field = "app_class", value_debug = ?other, "parse_event_payload: expected string for app_class");
                    return None;
                }
            };
            let title_str = match &f[EVENT_FIELD_TITLE] {
                Value::Str(s) => s.as_str(),
                other => {
                    warn!(?uid, tag = "Block", field = "title", value_debug = ?other, "parse_event_payload: expected string for title");
                    return None;
                }
            };
            let aid = match wellbeing_core::AppClass::new(app_class_str) {
                Ok(a) => a,
                Err(e) => {
                    warn!(?uid, tag = "Block", app_class = app_class_str, error = %e, "parse_event_payload: AppClass validation failed");
                    return None;
                }
            };
            Some(PlatformEvent::Block {
                app_class: aid,
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
                other => {
                    warn!(?uid, power_tag_debug = ?other, "parse_event_payload: expected known power tag (0=suspend,1=hibernate,2=shutdown)");
                    return None;
                }
            };
            Some(PlatformEvent::PowerEvent { kind, uid })
        }
        tag => {
            warn!(?uid, tag_debug = ?tag, "parse_event_payload: unknown event tag");
            None
        }
    }
}
