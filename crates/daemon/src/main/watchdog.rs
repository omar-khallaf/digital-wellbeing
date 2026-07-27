//! D-Bus name-loss watchdog.
//!
//! Monitors `NameOwnerChanged` on the system/session bus. When the daemon's
//! well-known name is lost, attempts re-acquisition on a fresh connection with
//! exponential backoff (up to 5 retries).

use std::sync::Arc;

use futures::StreamExt;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};
use wellbeing_core::SystemClock;
use wellbeing_core::dbus_constants::{DAEMON_BUS_NAME, DAEMON_OBJECT_PATH};
use wellbeing_daemon::{
    blocking::InternalEvent, bus_resolution::BusMode, dbus::DaemonInterface,
    dbus::DaemonInterfaceConfig, dbus::domain::BlockedAppsMap, platform::PlatformEvent,
    platform::linux::PluginRegistry, store::DbPool,
};

#[zbus::proxy(
    default_service = "org.freedesktop.DBus",
    default_path = "/org/freedesktop/DBus",
    interface = "org.freedesktop.DBus"
)]
trait DBusFdo {
    #[zbus(signal)]
    fn name_owner_changed(
        &self,
        name: String,
        old_owner: String,
        new_owner: String,
    ) -> zbus::Result<()>;
    #[zbus(signal)]
    fn name_lost(&self, name: String) -> zbus::Result<()>;
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn spawn(
    bus_mode: BusMode,
    recovery_pool: DbPool,
    recovery_registry: Arc<RwLock<PluginRegistry>>,
    recovery_event_tx: tokio::sync::mpsc::UnboundedSender<PlatformEvent>,
    recovery_policy_tx: tokio::sync::mpsc::Sender<InternalEvent>,
    recovery_blocked_apps: BlockedAppsMap,
    serving_state: Arc<RwLock<Option<(zbus::Connection, String)>>>,
    shutdown_token: CancellationToken,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        run(
            bus_mode,
            recovery_pool,
            recovery_registry,
            recovery_event_tx,
            recovery_policy_tx,
            recovery_blocked_apps,
            serving_state,
            shutdown_token,
        )
        .await;
    })
}

#[allow(clippy::too_many_arguments)]
async fn run(
    bus_mode: BusMode,
    recovery_pool: DbPool,
    recovery_registry: Arc<RwLock<PluginRegistry>>,
    recovery_event_tx: tokio::sync::mpsc::UnboundedSender<PlatformEvent>,
    recovery_policy_tx: tokio::sync::mpsc::Sender<InternalEvent>,
    recovery_blocked_apps: BlockedAppsMap,
    serving_state: Arc<RwLock<Option<(zbus::Connection, String)>>>,
    shutdown_token: CancellationToken,
) {
    loop {
        let watch_conn = match connect_to_bus(bus_mode).await {
            Some(c) => c,
            None => {
                tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                continue;
            }
        };

        let proxy = match DBusFdoProxy::new(&watch_conn).await {
            Ok(p) => p,
            Err(e) => {
                error!(%e, "watchdog: failed to create DBus proxy");
                tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                continue;
            }
        };

        let mut stream = match proxy.receive_name_owner_changed().await {
            Ok(s) => s,
            Err(e) => {
                error!(%e, "watchdog: failed to subscribe to NameOwnerChanged signal");
                tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                continue;
            }
        };

        loop {
            tokio::select! {
                msg = stream.next() => {
                    match msg {
                        Some(msg) => {
                            match msg.args() {
                                Ok(args) if args.name == DAEMON_BUS_NAME => {
                                    attempt_recovery(
                                        DAEMON_BUS_NAME,
                                        args.old_owner,
                                        args.new_owner,
                                        &serving_state,
                                        bus_mode,
                                        &recovery_pool,
                                        &recovery_registry,
                                        &recovery_event_tx,
                                        &recovery_policy_tx,
                                        &recovery_blocked_apps,
                                        &shutdown_token,
                                    ).await;
                                }
                                Ok(_) => {}
                                Err(e) => {
                                    error!(%e, "watchdog: failed to parse NameOwnerChanged args");
                                }
                            }
                        }
                        None => break,
                    }
                }
                _ = shutdown_token.cancelled() => {
                    info!("watchdog: shutting down");
                    return;
                }
            }
        }

        warn!("NameOwnerChanged stream ended, restarting watch in 1s");
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
    }
}

#[allow(clippy::too_many_arguments)]
async fn attempt_recovery(
    name: &str,
    old_owner: String,
    new_owner: String,
    serving_state: &Arc<RwLock<Option<(zbus::Connection, String)>>>,
    bus_mode: BusMode,
    recovery_pool: &DbPool,
    recovery_registry: &Arc<RwLock<PluginRegistry>>,
    recovery_event_tx: &tokio::sync::mpsc::UnboundedSender<PlatformEvent>,
    recovery_policy_tx: &tokio::sync::mpsc::Sender<InternalEvent>,
    recovery_blocked_apps: &BlockedAppsMap,
    shutdown_token: &CancellationToken,
) {
    let current_name = serving_state
        .read()
        .await
        .as_ref()
        .map(|(_, name)| name.clone());
    let Some(our_name) = current_name else { return };
    if old_owner != our_name {
        return;
    }

    error!(
        name = ?name,
        old_owner = ?old_owner,
        new_owner = ?new_owner,
        "D-Bus name lost, attempting re-acquisition"
    );

    let mut retries = 0u32;
    loop {
        if shutdown_token.is_cancelled() {
            info!("watchdog: re-acquisition cancelled");
            return;
        }

        let fresh_conn = match connect_to_bus(bus_mode).await {
            Some(c) => c,
            None => {
                retries += 1;
                if should_shutdown(retries, shutdown_token).await {
                    return;
                }
                continue;
            }
        };

        let fresh_interface = DaemonInterface::new(DaemonInterfaceConfig {
            policy_repo: wellbeing_daemon::policy::data::PolicyRepo::new(recovery_pool.clone()),
            categorization_repo: wellbeing_daemon::categorization::data::CategorizationRepo::new(
                recovery_pool.clone(),
            ),
            reports_repo: wellbeing_daemon::reports::data::ReportsRepo::new(recovery_pool.clone()),
            registry: recovery_registry.clone(),
            event_tx: recovery_event_tx.clone(),
            clock: Box::new(SystemClock),
            blocked_apps: recovery_blocked_apps.clone(),
            tokio_handle: tokio::runtime::Handle::current(),
            policy_tx: recovery_policy_tx.clone(),
        });

        if let Err(e) = fresh_conn
            .object_server()
            .at(DAEMON_OBJECT_PATH, fresh_interface)
            .await
        {
            error!(%e, "watchdog: failed to register object server on fresh connection");
            retries += 1;
            if should_shutdown(retries, shutdown_token).await {
                return;
            }
            continue;
        }

        match fresh_conn.request_name(DAEMON_BUS_NAME).await {
            Ok(_) => {
                let new_unique = fresh_conn
                    .unique_name()
                    .map(|n| n.to_string())
                    .unwrap_or_default();
                info!(unique_name = ?new_unique, "D-Bus name re-acquired on fresh connection");

                tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

                if fresh_conn.unique_name().is_some() {
                    info!(unique_name = ?new_unique, "D-Bus connection stable after re-acquisition");
                    *serving_state.write().await = Some((fresh_conn, new_unique));
                    return;
                }

                error!("fresh connection lost unique name during stabilization, retrying");
                retries += 1;
                if should_shutdown(retries, shutdown_token).await {
                    return;
                }
            }
            Err(e) => {
                error!(%e, "watchdog: failed to re-acquire D-Bus name");
                retries += 1;
                if should_shutdown(retries, shutdown_token).await {
                    return;
                }
            }
        }
    }
}

/// Returns `true` when retries are exhausted (caller should return immediately).
/// Returns `false` after applying exponential backoff delay (caller should retry).
async fn should_shutdown(retries: u32, shutdown_token: &CancellationToken) -> bool {
    if retries >= 5 {
        error!("max retries exceeded, shutting down");
        shutdown_token.cancel();
        true
    } else {
        tokio::time::sleep(tokio::time::Duration::from_millis(500 * retries as u64)).await;
        false
    }
}

async fn connect_to_bus(bus_mode: BusMode) -> Option<zbus::Connection> {
    match bus_mode {
        BusMode::System => zbus::Connection::system().await.ok(),
        BusMode::Session => zbus::Connection::session().await.ok(),
    }
}
