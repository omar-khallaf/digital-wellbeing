use std::collections::HashMap;

use futures::StreamExt;

use tracing::{error, info, warn};

use crate::platform::PlatformEvent;
use wellbeing_core::{
    AppId, PluginInstanceId, Uid,
    dbus_constants::{
        ACTIVITY_TAG_IDLE, FOCUS_FIELD_APP_ID, FOCUS_FIELD_OVERLAY, FOCUS_FIELD_PID,
        FOCUS_FIELD_TAG, FOCUS_FIELD_TITLE, FOCUS_FIELD_UID, FOCUS_STRUCT_FIELD_COUNT,
        FOCUS_TAG_APP, FOCUS_TAG_DESKTOP,
    },
};
use zbus::proxy;
use zvariant::OwnedValue;

#[proxy(
    interface = "org.wellbeing.v1.Manager",
    default_path = "/org/wellbeing/Manager"
)]
pub trait Manager {
    /// Current focus (property `CurrentFocus` on the wire).
    ///
    /// Returns the same variant encoding as `FocusChanged` — either
    /// `U32(0)` for desktop (no app) or a `Structure` with app fields.
    #[zbus(property, name = "CurrentFocus")]
    fn current_focus(&self) -> zbus::Result<OwnedValue>;

    #[zbus(signal)]
    fn user_action(&self, app_id: &str, action: u32) -> zbus::Result<()>;

    #[zbus(signal)]
    fn focus_changed(&self, window: OwnedValue) -> zbus::Result<()>;

    #[zbus(signal)]
    fn activity_changed(&self, tag: u32) -> zbus::Result<()>;
}

pub struct ManagerClient {
    pub uid: Uid,
    proxy: ManagerProxy<'static>,
}

impl ManagerClient {
    pub fn new(uid: Uid, proxy: ManagerProxy<'static>) -> Self {
        Self { uid, proxy }
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
        self.clients.insert(instance_id, client);
        info!(?uid, "plugin registered");
    }

    pub fn unregister(&mut self, instance_id: &PluginInstanceId) {
        if let Some(client) = self.clients.remove(instance_id) {
            self.by_uid.remove(&client.uid);
            info!(?client.uid, "plugin unregistered");
        }
    }

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
            let mut action_stream = match proxy.receive_user_action().await {
                Ok(s) => s,
                Err(e) => {
                    error!("failed to subscribe user_action: {e}");
                    return;
                }
            };

            loop {
                tokio::select! {
                    Some(signal) = focus_stream.next() => {
                        if let Ok(args) = signal.args() {
                            let val: zvariant::OwnedValue = args.window;
                            use zvariant::Value;
                            let v: Value = val.into();
                            match &v {
                                Value::U32(FOCUS_TAG_DESKTOP) => {
                                    tx.send(PlatformEvent::Unfocused).await.ok();
                                }
                                Value::Structure(s) if s.fields().len() >= FOCUS_STRUCT_FIELD_COUNT => {
                                    let f = s.fields();
                                    if let (
                                        Value::U32(FOCUS_TAG_APP),
                                        Value::Str(app_id),
                                        Value::Str(title),
                                        Value::U32(pid),
                                        Value::U32(uid),
                                        Value::Bool(overlay),
                                    ) = (&f[FOCUS_FIELD_TAG], &f[FOCUS_FIELD_APP_ID], &f[FOCUS_FIELD_TITLE], &f[FOCUS_FIELD_PID], &f[FOCUS_FIELD_UID], &f[FOCUS_FIELD_OVERLAY])
                                        && let Ok(aid) = wellbeing_core::AppId::new(app_id.as_str()) {
                                            let wt = wellbeing_core::WindowTitle::new(title.as_str());
                                            tx.send(PlatformEvent::WindowFocused {
                                                app_id: aid,
                                                title: wt,
                                                pid: wellbeing_core::Pid(*pid),
                                                uid: wellbeing_core::Uid(*uid),
                                                overlay_shown: *overlay,
                                            }).await.ok();
                                        }
                                }
                                _ => {}
                            }
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
                    Some(signal) = action_stream.next() => {
                        if let Ok(args) = signal.args()
                            && let Ok(aid) = AppId::new(args.app_id) {
                                tx.send(PlatformEvent::UserAction {
                                    app_id: aid,
                                    action: args.action,
                                    uid,
                                }).await.ok();
                            }
                    }
                    else => break,
                }
            }
        });

        Some(rx)
    }
}

impl PluginRegistry {
    /// Query the `CurrentFocus` property from every registered plugin.
    ///
    /// Returns the first [`PlatformEvent::WindowFocused`] whose variant
    /// carries an app window, or `None` when all plugins report desktop
    /// focus (no app) or are unreachable.
    pub async fn query_current_focus(&self) -> Option<PlatformEvent> {
        for (instance_id, client) in &self.clients {
            let val = match client.proxy.current_focus().await {
                Ok(v) => v,
                Err(e) => {
                    error!(
                        plugin = %instance_id.as_ref(),
                        uid = ?client.uid,
                        error = %e,
                        "query_current_focus failed"
                    );
                    continue;
                }
            };
            let v: zvariant::Value = val.into();
            // D-Bus properties always carry a variant wrapper (sig `v`).
            let v = match v {
                zvariant::Value::Value(boxed) => *boxed,
                other => other,
            };
            match v {
                zvariant::Value::U32(FOCUS_TAG_DESKTOP) => {
                    continue;
                }
                zvariant::Value::Structure(s) if s.fields().len() >= FOCUS_STRUCT_FIELD_COUNT => {
                    let f = s.fields();
                    if let (
                        zvariant::Value::U32(FOCUS_TAG_APP),
                        zvariant::Value::Str(app_id),
                        zvariant::Value::Str(title),
                        zvariant::Value::U32(pid),
                        zvariant::Value::U32(uid),
                        zvariant::Value::Bool(overlay),
                    ) = (
                        &f[FOCUS_FIELD_TAG],
                        &f[FOCUS_FIELD_APP_ID],
                        &f[FOCUS_FIELD_TITLE],
                        &f[FOCUS_FIELD_PID],
                        &f[FOCUS_FIELD_UID],
                        &f[FOCUS_FIELD_OVERLAY],
                    ) && let Ok(aid) = AppId::new(app_id.as_str())
                    {
                        return Some(PlatformEvent::WindowFocused {
                            app_id: aid,
                            title: wellbeing_core::WindowTitle::new(title.as_str()),
                            pid: wellbeing_core::Pid(*pid),
                            uid: wellbeing_core::Uid(*uid),
                            overlay_shown: *overlay,
                        });
                    }
                }
                _ => {}
            }
        }
        None
    }
}

impl Default for PluginRegistry {
    fn default() -> Self {
        Self::new()
    }
}
