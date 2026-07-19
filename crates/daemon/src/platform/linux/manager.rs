use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use ed25519_dalek::{Signature, Signer, SigningKey};
use futures::StreamExt;

use tracing::{error, info, warn};
use uuid::Uuid;
use wellbeing_core::{AppId, PluginInstanceId, PolicyId, Uid};
use zbus::proxy;
use zvariant::{OwnedValue, Type};

use crate::platform::{OverlayConfig, PlatformEvent};

#[derive(Debug, Clone, Type)]
#[zvariant(signature = "v")]
pub struct RawUserAction {
    pub app_id: String,
    pub action: u32,
    pub policy_id: u64,
    pub blocked_since: u64,
    pub signature: Vec<u8>,
}

#[proxy(
    interface = "org.wellbeing.v1.Manager",
    default_path = "/org/wellbeing/Manager"
)]
pub trait Manager {
    async fn overlay(&self, envelope: &OwnedValue) -> zbus::Result<bool>;

    #[zbus(property)]
    fn current_session(&self) -> zbus::Result<OwnedValue>;

    #[zbus(signal)]
    fn user_action(
        &self,
        app_id: &str,
        action: u32,
        policy_id: u64,
        blocked_since: u64,
        signature: &[u8],
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    fn focus_changed(&self, window: OwnedValue) -> zbus::Result<()>;

    #[zbus(signal)]
    fn activity_changed(&self, idle: bool) -> zbus::Result<()>;
}

#[derive(Debug, Clone, Type)]
#[allow(dead_code)]
pub struct SignedEnvelope {
    pub payload: OwnedValue,
    pub issued_at: u64,
    pub signature: Vec<u8>,
}

pub struct ManagerClient {
    pub instance_id: PluginInstanceId,
    pub uid: Uid,
    proxy: ManagerProxy<'static>,
}

impl ManagerClient {
    pub fn new(instance_id: PluginInstanceId, uid: Uid, proxy: ManagerProxy<'static>) -> Self {
        Self {
            instance_id,
            uid,
            proxy,
        }
    }

    pub async fn show_overlay(&self, config: &OverlayConfig, keypair: &SigningKey) -> Result<()> {
        let blocked_since_ms = config
            .blocked_since
            .duration_since(UNIX_EPOCH)
            .context("blocked_since before epoch")?
            .as_millis() as u64;

        let token_msg = [
            config.app_id.as_ref().as_bytes(),
            &config.policy_id.0.to_be_bytes(),
            &blocked_since_ms.to_be_bytes(),
            self.instance_id.as_ref().as_bytes(),
        ]
        .concat();
        let token_sig = keypair.sign(&token_msg);

        let show_payload = {
            use zvariant::StructureBuilder;
            let mut sb = StructureBuilder::new();
            sb = sb.add_field("show".to_string());
            sb = sb.add_field(config.app_id.as_ref().to_string());
            sb = sb.add_field(config.policy_id.0);
            sb = sb.add_field(config.reason as u32);
            sb = sb.add_field(blocked_since_ms);
            sb = sb.add_field::<Vec<u32>>(
                config.available_actions.iter().map(|a| *a as u32).collect(),
            );
            sb = sb.add_field(token_sig.to_bytes().to_vec());
            let s = sb.build().expect("structure build");
            zvariant::Value::new(s)
        };

        let envelope = Self::sign_envelope(show_payload, keypair)?;
        self.proxy.overlay(&envelope).await?;
        Ok(())
    }

    pub async fn hide_overlay(&self, app_id: &AppId, keypair: &SigningKey) -> Result<()> {
        let hide_payload = {
            use zvariant::StructureBuilder;
            let mut sb = StructureBuilder::new();
            sb = sb.add_field("hide".to_string());
            sb = sb.add_field(app_id.as_ref().to_string());
            let s = sb.build().expect("structure build");
            zvariant::Value::new(s)
        };
        let envelope = Self::sign_envelope(hide_payload, keypair)?;
        self.proxy.overlay(&envelope).await?;
        Ok(())
    }

    fn sign_envelope(payload: zvariant::Value<'_>, keypair: &SigningKey) -> Result<OwnedValue> {
        let issued_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .context("system time before epoch")?
            .as_millis() as u64;

        use zvariant::Endian;
        use zvariant::serialized::Context;
        let ctxt = Context::new_dbus(Endian::Little, 0);
        let payload_bytes =
            zvariant::to_bytes(ctxt, &payload).context("failed to serialize payload")?;
        let sig =
            keypair.sign(&[payload_bytes.as_ref(), issued_at.to_be_bytes().as_ref()].concat());

        use zvariant::StructureBuilder;
        let envelope = StructureBuilder::new()
            .add_field(payload)
            .add_field(issued_at)
            .add_field(sig.to_bytes().to_vec())
            .build()
            .context("structure build")?;
        zvariant::Value::new(envelope)
            .try_into()
            .context("failed to convert envelope")
    }
}

pub struct PluginRegistry {
    clients: HashMap<PluginInstanceId, ManagerClient>,
    by_uid: HashMap<Uid, PluginInstanceId>,
    keypair: SigningKey,
    key_id: String,
}

impl PluginRegistry {
    pub fn new() -> Self {
        let mut seed = [0u8; 32];
        getrandom::fill(&mut seed).expect("getrandom for key seed");
        let keypair = SigningKey::from_bytes(&seed);
        let key_id = Uuid::new_v4().to_string();
        Self {
            clients: HashMap::new(),
            by_uid: HashMap::new(),
            keypair,
            key_id,
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
        let client = ManagerClient::new(instance_id.clone(), uid, proxy);
        self.clients.insert(instance_id, client);
        info!(?uid, "plugin registered");
    }

    pub fn unregister(&mut self, instance_id: &PluginInstanceId) {
        if let Some(client) = self.clients.remove(instance_id) {
            self.by_uid.remove(&client.uid);
            info!(?client.uid, "plugin unregistered");
        }
    }

    pub async fn show_overlay_for(&self, config: &OverlayConfig, uid: Uid) -> Result<()> {
        let client = self
            .clients
            .values()
            .find(|c| c.uid == uid)
            .ok_or_else(|| anyhow::anyhow!("no plugin for uid {}", uid.0))?;
        client.show_overlay(config, &self.keypair).await
    }

    pub async fn hide_overlay_for(&self, app_id: &AppId, uid: Uid) -> Result<()> {
        let client = self
            .clients
            .values()
            .find(|c| c.uid == uid)
            .ok_or_else(|| anyhow::anyhow!("no plugin for uid {}", uid.0))?;
        client.hide_overlay(app_id, &self.keypair).await
    }

    pub fn verify_user_action(
        &self,
        ev: &RawUserAction,
        instance_id: &PluginInstanceId,
    ) -> Option<(AppId, u32, PolicyId)> {
        let msg = [
            ev.app_id.as_bytes(),
            &ev.policy_id.to_be_bytes(),
            &ev.blocked_since.to_be_bytes(),
            instance_id.as_ref().as_bytes(),
        ]
        .concat();
        let sig = Signature::from_slice(&ev.signature).ok()?;
        if self.keypair.verify(&msg, &sig).is_err() {
            warn!("user_action token verification failed");
            return None;
        }
        Some((
            AppId::new(&ev.app_id).ok()?,
            ev.action,
            PolicyId(ev.policy_id as i64),
        ))
    }

    pub fn daemon_public_key(&self) -> (String, Vec<u8>) {
        (
            self.key_id.clone(),
            self.keypair.verifying_key().to_bytes().to_vec(),
        )
    }

    pub fn keypair(&self) -> &SigningKey {
        &self.keypair
    }

    pub async fn subscribe_signals(
        &self,
        instance_id: &PluginInstanceId,
    ) -> Option<tokio::sync::mpsc::Receiver<PlatformEvent>> {
        let client = self.clients.get(instance_id)?;
        let proxy = client.proxy.clone();
        let (tx, rx) = tokio::sync::mpsc::channel::<PlatformEvent>(256);
        let instance_id = instance_id.clone();
        let keypair = self.keypair.clone();

        tokio::spawn(async move {
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
                                Value::U32(1) => {
                                    tx.send(PlatformEvent::Unfocused).await.ok();
                                }
                                Value::Structure(s) if s.fields().len() >= 6 => {
                                    let f = s.fields();
                                    if let (
                                        Value::U32(2),
                                        Value::Str(app_id),
                                        Value::Str(title),
                                        Value::U32(pid),
                                        Value::U32(uid),
                                        Value::Bool(overlay),
                                    ) = (&f[0], &f[1], &f[2], &f[3], &f[4], &f[5])
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
                            let event = if args.idle {
                                PlatformEvent::Idle
                            } else {
                                PlatformEvent::Resumed
                            };
                            tx.send(event).await.ok();
                        }
                    }
                    Some(signal) = action_stream.next() => {
                        if let Ok(args) = signal.args() {
                            let msg = [
                                args.app_id.as_bytes(),
                                &args.policy_id.to_be_bytes(),
                                &args.blocked_since.to_be_bytes(),
                                instance_id.as_ref().as_bytes(),
                            ]
                            .concat();
                            if let Ok(sig) = Signature::from_slice(args.signature) {
                                if keypair.verify(&msg, &sig).is_ok() {
                                    if let Ok(aid) = AppId::new(args.app_id) {
                                        tx.send(PlatformEvent::UserAction {
                                            app_id: aid,
                                            action: args.action,
                                            policy_id: PolicyId(args.policy_id as i64),
                                        })
                                        .await.ok();
                                    }
                                } else {
                                    warn!("user_action signal verification failed");
                                }
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
