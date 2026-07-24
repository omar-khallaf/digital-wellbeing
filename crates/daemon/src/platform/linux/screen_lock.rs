//! Screen lock/unlock detection via `org.gnome.ScreenSaver`.
//!
//! [`ScreenLockWatcher`] subscribes to the `ActiveChanged` signal on the
//! session bus and emits [`ScreenLockEvent`]s.  On unlock the consumer
//! SHOULD query the plugin's `CurrentFocus` property via
//! [`super::PluginRegistry::query_current_focus`] to re-inject a
//! `WindowFocused` event into the event stream.
//!
//! # Lock-state flag
//!
//! [`ScreenLockWatcher::watch`] also returns an [`Arc<AtomicBool>`] that
//! tracks whether the screen is currently locked.  Consumers can check
//! this flag — for example, to skip a focus-reconciliation on system
//! resume when the screen is still locked, deferring to the unlock
//! handler instead.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::Result;
use futures::StreamExt;
use tracing::error;
use zbus::proxy;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenLockEvent {
    Locked,
    Unlocked,
}

#[proxy(
    interface = "org.gnome.ScreenSaver",
    default_service = "org.gnome.ScreenSaver",
    default_path = "/org/gnome/ScreenSaver"
)]
trait ScreenSaver {
    #[zbus(signal)]
    fn active_changed(&self, active: bool) -> zbus::Result<()>;
}

pub struct ScreenLockWatcher;

impl ScreenLockWatcher {
    /// Connect to the session bus and subscribe to
    /// `org.gnome.ScreenSaver.ActiveChanged`.
    ///
    /// Returns a receiver that yields [`ScreenLockEvent`]s **and** a shared
    /// flag that is `true` while the screen is locked.
    ///
    /// The flag defaults to `false` (unlocked) and is updated atomically on
    /// every `ActiveChanged` signal.  Consumers can use it to avoid
    /// unnecessary work when the screen is known to be locked.
    pub async fn watch() -> Result<(
        tokio::sync::mpsc::UnboundedReceiver<ScreenLockEvent>,
        Arc<AtomicBool>,
    )> {
        let conn = zbus::Connection::session()
            .await
            .map_err(|e| anyhow::anyhow!("failed to connect to session bus: {e}"))?;

        let proxy = ScreenSaverProxy::new(&conn)
            .await
            .map_err(|e| anyhow::anyhow!("failed to get ScreenSaver proxy: {e}"))?;

        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let is_locked = Arc::new(AtomicBool::new(false));
        let flag = is_locked.clone();

        tokio::spawn(async move {
            let mut stream = match proxy.receive_active_changed().await {
                Ok(s) => s,
                Err(e) => {
                    error!("cannot subscribe to ActiveChanged: {e}");
                    return;
                }
            };

            while let Some(signal) = stream.next().await {
                if let Ok(args) = signal.args() {
                    if *args.active() {
                        flag.store(true, Ordering::Release);
                        tx.send(ScreenLockEvent::Locked).ok();
                    } else {
                        flag.store(false, Ordering::Release);
                        tx.send(ScreenLockEvent::Unlocked).ok();
                    }
                }
            }
        });

        Ok((rx, is_locked))
    }
}
