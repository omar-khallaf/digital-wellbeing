use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use futures::Stream;
use tokio::sync::RwLock;
use zbus::proxy;
use zvariant::Value;

use crate::bus_resolution::BusMode;
use crate::platform::{Platform, PlatformEvent};

mod manager;
mod screen_lock;
mod suspend;

pub use manager::{ManagerProxy, PluginRegistry};
pub use screen_lock::{ScreenLockEvent, ScreenLockWatcher};
pub use suspend::{PowerEvent, PowerStateWatcher};

#[proxy(
    interface = "org.freedesktop.Notifications",
    default_service = "org.freedesktop.Notifications",
    default_path = "/org/freedesktop/Notifications"
)]
trait Notifications {
    async fn notify(
        &self,
        app_name: &str,
        replaces_id: u32,
        app_icon: &str,
        summary: &str,
        body: &str,
        actions: &[&str],
        hints: HashMap<String, Value<'_>>,
        expire_timeout: i32,
    ) -> zbus::Result<u32>;
}

pub struct LinuxPlatform {
    registry: Arc<RwLock<PluginRegistry>>,
    event_tx: tokio::sync::mpsc::UnboundedSender<PlatformEvent>,
    session_conn: zbus::Connection,
}

impl Platform for LinuxPlatform {
    type EventStream = tokio_stream::wrappers::UnboundedReceiverStream<PlatformEvent>;

    async fn notify(&self, title: &str, body: &str) -> Result<()> {
        let proxy = NotificationsProxy::new(&self.session_conn).await?;
        proxy
            .notify(
                "digital-wellbeing",
                0,
                "",
                title,
                body,
                &[],
                HashMap::new(),
                -1,
            )
            .await?;
        Ok(())
    }
}

impl LinuxPlatform {
    pub fn registry(&self) -> Arc<RwLock<PluginRegistry>> {
        self.registry.clone()
    }

    pub fn event_tx(&self) -> tokio::sync::mpsc::UnboundedSender<PlatformEvent> {
        self.event_tx.clone()
    }
}

pub struct LinuxPlatformBuilder;

impl LinuxPlatformBuilder {
    pub fn new() -> Self {
        Self
    }

    pub async fn build(
        self,
        bus_mode: BusMode,
    ) -> Result<(
        LinuxPlatform,
        impl Stream<Item = PlatformEvent> + Send + 'static,
    )> {
        let (event_tx, event_rx) = tokio::sync::mpsc::unbounded_channel();
        let registry = Arc::new(RwLock::new(PluginRegistry::new()));

        let session_conn = match bus_mode {
            BusMode::System => zbus::Connection::system().await?,
            BusMode::Session => zbus::Connection::session().await?,
        };

        let platform = LinuxPlatform {
            registry,
            event_tx,
            session_conn,
        };

        Ok((
            platform,
            tokio_stream::wrappers::UnboundedReceiverStream::new(event_rx),
        ))
    }
}

impl Default for LinuxPlatformBuilder {
    fn default() -> Self {
        Self::new()
    }
}
