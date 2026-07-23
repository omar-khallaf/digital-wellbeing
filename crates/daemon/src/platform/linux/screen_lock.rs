//! Screen lock/unlock detection via `org.gnome.ScreenSaver`.
//!
//! [`ScreenLockWatcher`] subscribes to the `ActiveChanged` signal on the
//! session bus and emits [`ScreenLockEvent`]s.  On unlock the consumer
//! SHOULD query the plugin's `CurrentFocus` property via
//! [`super::PluginRegistry::query_current_focus`] to re-inject a
//! `WindowFocused` event into the event stream.

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
    /// Returns a receiver that yields [`ScreenLockEvent`]s.
    pub async fn watch() -> Result<tokio::sync::mpsc::UnboundedReceiver<ScreenLockEvent>> {
        let conn = zbus::Connection::session()
            .await
            .map_err(|e| anyhow::anyhow!("failed to connect to session bus: {e}"))?;

        let proxy = ScreenSaverProxy::new(&conn)
            .await
            .map_err(|e| anyhow::anyhow!("failed to get ScreenSaver proxy: {e}"))?;

        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();

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
                        tx.send(ScreenLockEvent::Locked).ok();
                    } else {
                        tx.send(ScreenLockEvent::Unlocked).ok();
                    }
                }
            }
        });

        Ok(rx)
    }
}
