use anyhow::Result;
use futures::StreamExt;
use tracing::{error, info};
use zbus::proxy;

#[proxy(
    interface = "org.freedesktop.login1.Manager",
    default_service = "org.freedesktop.login1",
    default_path = "/org/freedesktop/login1"
)]
trait LoginManager {
    fn inhibit(
        &self,
        what: &str,
        who: &str,
        why: &str,
        mode: &str,
    ) -> zbus::Result<zbus::zvariant::OwnedFd>;

    #[zbus(signal)]
    fn prepare_for_sleep(&self, start: bool) -> zbus::Result<()>;

    #[zbus(signal)]
    fn prepare_for_shutdown(&self, start: bool) -> zbus::Result<()>;

    #[zbus(signal)]
    fn session_removed(
        &self,
        session: &str,
        object_path: zvariant::ObjectPath<'_>,
    ) -> zbus::Result<()>;
}

/// Power-state events from logind.
///
/// Screen lock/unlock is handled separately by [`super::ScreenLockWatcher`]
/// via `org.gnome.ScreenSaver` — see `screen_lock.rs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerEvent {
    Slept,
    ShutDown,
    LoggedOut,
    ResumedSystem,
}

pub struct PowerStateWatcher;

impl PowerStateWatcher {
    pub async fn watch() -> Result<tokio::sync::mpsc::UnboundedReceiver<PowerEvent>> {
        let conn = zbus::Connection::system()
            .await
            .map_err(|e| anyhow::anyhow!("failed to connect to system bus: {e}"))?;

        let manager = LoginManagerProxy::new(&conn)
            .await
            .map_err(|e| anyhow::anyhow!("failed to get login manager: {e}"))?;

        // Resolve the current session path via the /self symlink
        // so we can detect when *our* session is removed (logged out).
        let session_path = {
            use zbus::proxy::Builder;
            let p: zbus::Proxy<'_> = Builder::new(&conn)
                .destination("org.freedesktop.login1")?
                .path("/org/freedesktop/login1/session/self")?
                .interface("org.freedesktop.login1.Session")?
                .build()
                .await?;
            p.path().to_string()
        };

        let _inhibit_fd = manager
            .inhibit(
                "sleep:shutdown",
                "digital-wellbeing",
                "Flush session data before power state change",
                "delay",
            )
            .await
            .map_err(|e| anyhow::anyhow!("failed to inhibit logind: {e}"))?;

        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();

        tokio::spawn(async move {
            let mut sleep_stream = match manager.receive_prepare_for_sleep().await {
                Ok(s) => s,
                Err(e) => {
                    error!("cannot subscribe prepare_for_sleep: {e}");
                    return;
                }
            };
            let mut shutdown_stream = match manager.receive_prepare_for_shutdown().await {
                Ok(s) => s,
                Err(e) => {
                    error!("cannot subscribe prepare_for_shutdown: {e}");
                    return;
                }
            };
            let mut session_removed_stream = match manager.receive_session_removed().await {
                Ok(s) => s,
                Err(e) => {
                    error!("cannot subscribe session_removed: {e}");
                    return;
                }
            };

            loop {
                tokio::select! {
                    Some(signal) = sleep_stream.next() => {
                        if let Ok(args) = signal.args() {
                            if *args.start() {
                                info!("logind: prepare_for_sleep");
                                tx.send(PowerEvent::Slept).ok();
                            } else {
                                info!("logind: resumed from sleep");
                                tx.send(PowerEvent::ResumedSystem).ok();
                            }
                        }
                    }
                    Some(signal) = shutdown_stream.next() => {
                        if let Ok(args) = signal.args()
                            && *args.start() {
                            info!("logind: prepare_for_shutdown");
                            tx.send(PowerEvent::ShutDown).ok();
                        }
                    }
                    Some(signal) = session_removed_stream.next() => {
                        if let Ok(args) = signal.args()
                            && args.object_path.to_string() == session_path
                        {
                            info!("logind: session removed (logged out)");
                            tx.send(PowerEvent::LoggedOut).ok();
                        }
                    }
                }
            }
        });

        Ok(rx)
    }
}
