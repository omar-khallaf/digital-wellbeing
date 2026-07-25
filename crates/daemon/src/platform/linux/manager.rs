use std::collections::HashMap;

use futures::StreamExt;

use tracing::{error, info, warn};

use crate::platform::PlatformEvent;
use wellbeing_core::{
    PluginInstanceId, Uid,
    dbus_constants::{
        ACTIVITY_TAG_IDLE, FOCUS_FIELD_APP_ID, FOCUS_FIELD_PID, FOCUS_FIELD_TAG, FOCUS_FIELD_TITLE,
        FOCUS_FIELD_UID, FOCUS_STRUCT_FIELD_COUNT, FOCUS_TAG_APP, FOCUS_TAG_BLOCKED,
        FOCUS_TAG_DESKTOP,
    },
};
use zbus::proxy;
use zvariant::OwnedValue;

#[proxy(
    interface = "org.wellbeing.v1.Manager",
    default_path = "/org/wellbeing/Manager"
)]
pub trait Manager {
    #[zbus(signal)]
    fn focus_changed(&self, window: OwnedValue) -> zbus::Result<()>;

    #[zbus(signal)]
    fn activity_changed(&self, tag: u32) -> zbus::Result<()>;

    /// Read-only property returning the same variant as FocusChanged —
    /// None (Desktop), WindowFocused, or WindowBlocked.
    #[zbus(property)]
    fn current_focus(&self) -> zbus::Result<OwnedValue>;
}

pub struct ManagerClient {
    pub uid: Uid,
    proxy: ManagerProxy<'static>,
}

impl ManagerClient {
    pub fn new(uid: Uid, proxy: ManagerProxy<'static>) -> Self {
        Self { uid, proxy }
    }

    /// Query the plugin's CurrentFocus D-Bus property and convert it to a
    /// PlatformEvent. Returns None if the property returns Desktop (no focus)
    /// or if deserialization fails.
    pub async fn current_focus(&self) -> Option<PlatformEvent> {
        let val: OwnedValue = self.proxy.current_focus().await.ok()?;
        parse_focus_variant(val)
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

    pub async fn subscribe_signals(
        &self,
        instance_id: &PluginInstanceId,
        handle: &tokio::runtime::Handle,
    ) -> Option<tokio::sync::mpsc::Receiver<PlatformEvent>> {
        let client = self.clients.get(instance_id)?;
        let proxy = client.proxy.clone();
        let (tx, rx) = tokio::sync::mpsc::channel::<PlatformEvent>(256);

        handle.spawn(async move {
            let mut focus_stream = match proxy.receive_focus_changed().await {
                Ok(s) => s,
                Err(e) => {
                    error!("failed to subscribe focus_changed: {e}");
                    return;
                }
            };
            let mut activity_stream = match proxy.receive_activity_changed().await {
                Ok(s) => s,
                Err(e) => {
                    error!("failed to subscribe activity_changed: {e}");
                    return;
                }
            };
            loop {
                tokio::select! {
                    Some(signal) = focus_stream.next() => {
                        if let Ok(args) = signal.args()
                            && let Some(event) = parse_focus_variant(args.window) {
                                tx.send(event).await.ok();
                            }
                    }
                    Some(signal) = activity_stream.next() => {
                        if let Ok(args) = signal.args() {
                            let event = if args.tag == ACTIVITY_TAG_IDLE {
                                PlatformEvent::IdleActivity
                            } else {
                                PlatformEvent::ResumedActivity
                            };
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

/// Parse a FocusChanged variant OwnedValue into a PlatformEvent.
/// Shared by the signal handler and the CurrentFocus property query.
fn parse_focus_variant(val: OwnedValue) -> Option<PlatformEvent> {
    use zvariant::Value;
    let v: Value = val.into();
    match &v {
        Value::U32(FOCUS_TAG_DESKTOP) => Some(PlatformEvent::Unfocused),
        Value::Structure(s) if s.fields().len() >= FOCUS_STRUCT_FIELD_COUNT => {
            let f = s.fields();
            if let (
                tag @ (Value::U32(FOCUS_TAG_APP) | Value::U32(FOCUS_TAG_BLOCKED)),
                Value::Str(app_id),
                Value::Str(title),
                Value::U32(pid),
                Value::U32(uid),
            ) = (
                &f[FOCUS_FIELD_TAG],
                &f[FOCUS_FIELD_APP_ID],
                &f[FOCUS_FIELD_TITLE],
                &f[FOCUS_FIELD_PID],
                &f[FOCUS_FIELD_UID],
            ) && let Ok(aid) = wellbeing_core::AppId::new(app_id.as_str())
            {
                let wt = wellbeing_core::WindowTitle::new(title.as_str());
                match tag {
                    Value::U32(FOCUS_TAG_BLOCKED) => Some(PlatformEvent::WindowBlocked {
                        app_id: aid,
                        title: wt,
                        uid: wellbeing_core::Uid(*uid),
                    }),
                    _ => Some(PlatformEvent::WindowFocused {
                        app_id: aid,
                        title: wt,
                        pid: wellbeing_core::Pid(*pid),
                        uid: wellbeing_core::Uid(*uid),
                    }),
                }
            } else {
                None
            }
        }
        _ => None,
    }
}
