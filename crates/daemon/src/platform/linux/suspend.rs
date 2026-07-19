use anyhow::Result;
use futures::StreamExt;
use tracing::{error, info};
use wellbeing_core::Clock;
use zbus::proxy;

use crate::store::DbPool;

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
}

#[proxy(
    interface = "org.freedesktop.login1.Session",
    default_service = "org.freedesktop.login1",
    default_path = "/org/freedesktop/login1/session/self"
)]
trait LoginSession {
    #[zbus(signal)]
    fn lock(&self) -> zbus::Result<()>;

    #[zbus(signal)]
    fn unlock(&self) -> zbus::Result<()>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerEvent {
    Slept,
    ShutDown,
    Locked,
    LoggedOut,
}

pub struct PowerStateWatcher;

impl PowerStateWatcher {
    pub async fn watch(
        pool: DbPool,
        clock: Box<dyn Clock>,
    ) -> Result<tokio::sync::mpsc::UnboundedReceiver<PowerEvent>> {
        let conn = zbus::Connection::system()
            .await
            .map_err(|e| anyhow::anyhow!("failed to connect to system bus: {e}"))?;

        let manager = LoginManagerProxy::new(&conn)
            .await
            .map_err(|e| anyhow::anyhow!("failed to get login manager: {e}"))?;

        let session = LoginSessionProxy::new(&conn)
            .await
            .map_err(|e| anyhow::anyhow!("failed to get login session: {e}"))?;

        // Acquire delay inhibitor for sleep + shutdown.
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
            let mut lock_stream = match session.receive_lock().await {
                Ok(s) => s,
                Err(e) => {
                    error!("cannot subscribe lock: {e}");
                    return;
                }
            };
            let mut unlock_stream = match session.receive_unlock().await {
                Ok(s) => s,
                Err(e) => {
                    error!("cannot subscribe unlock: {e}");
                    return;
                }
            };

            loop {
                tokio::select! {
                    Some(signal) = sleep_stream.next() => {
                        if let Ok(args) = signal.args()
                            && *args.start() {
                                info!("logind: prepare_for_sleep");
                                Self::flush_event(&pool, &*clock, 0).await;
                                tx.send(PowerEvent::Slept).ok();
                            }
                    }
                    Some(signal) = shutdown_stream.next() => {
                        if let Ok(args) = signal.args()
                            && *args.start() {
                                info!("logind: prepare_for_shutdown");
                                Self::flush_event(&pool, &*clock, 0).await;
                                tx.send(PowerEvent::ShutDown).ok();
                            }
                    }
                    Some(_) = lock_stream.next() => {
                        info!("logind: session locked");
                        Self::flush_event(&pool, &*clock, 0).await;
                        tx.send(PowerEvent::Locked).ok();
                    }
                    Some(_) = unlock_stream.next() => {
                        // Unlock is a no-op for intervals: next WindowFocused
                        // reopens a fresh interval.
                    }
                }
            }
        });

        Ok(rx)
    }

    async fn flush_event(pool: &DbPool, clock: &dyn Clock, user_id: i32) {
        use diesel::ExpressionMethods;
        use diesel_async::RunQueryDsl;
        let mut conn = match pool.get().await {
            Ok(c) => c,
            Err(e) => {
                error!("flush_event: pool error: {e}");
                return;
            }
        };
        let now = clock.now().format("%Y-%m-%d %H:%M:%S").to_string();
        let payload = serde_json::json!({"t": &now}).to_string();
        if let Err(e) = diesel::insert_into(crate::store::schema::events::table)
            .values((
                crate::store::schema::events::event_type.eq(4),
                crate::store::schema::events::payload.eq(&payload),
                crate::store::schema::events::user_id.eq(user_id),
            ))
            .execute(&mut conn)
            .await
        {
            error!("flush_event: write failed: {e}");
        }
    }
}
