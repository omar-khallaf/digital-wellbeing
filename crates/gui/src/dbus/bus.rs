//! D-Bus bus selection, daemon presence watching, and `BusManager`.
//!
//! Implements the 4-step resolution algorithm from 13-deployment-modes.md,
//! NameOwnerChanged watching for rapid daemon restart detection, and the
//! `BusManager` that owns dual-bus connections and creates fresh proxies.

use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use tokio::sync::Mutex;
use tokio::sync::broadcast;
use tracing::{debug, info, warn};
use wellbeing_core::dbus_constants::DAEMON_BUS_NAME;
use zbus::Connection;

use super::client::DaemonProxy;

/// Events from NameOwnerChanged watching for the daemon's bus name.
///
/// Replaces the 10-second polling approach — the D-Bus daemon pushes
/// name change signals immediately instead of the GUI polling every 10s.
///
/// `Appeared` carries the [`BusType`] so consumers can record the exact
/// bus without guessing (the watcher task knows which bus it's on).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DaemonPresenceEvent {
    Appeared(BusType),
    Disappeared,
}

/// Spawn background tasks on both busses watching `NameOwnerChanged` for
/// `DAEMON_BUS_NAME`. Returns a broadcast receiver so multiple flows can
/// independently subscribe to daemon presence changes.
///
/// Both busses are watched simultaneously so cross-bus restarts (daemon moves
/// from system to session bus or vice versa) are detected instantly.
pub fn spawn_daemon_presence_broadcast(
    sys_conn: &Connection,
    sess_conn: &Connection,
) -> broadcast::Receiver<DaemonPresenceEvent> {
    let (tx, rx) = broadcast::channel(16);

    let tx_sys = tx.clone();
    let sys = sys_conn.clone();
    tokio::spawn(async move {
        name_owner_watch_on_bus_broadcast(sys, BusType::System, tx_sys).await;
    });
    let sess = sess_conn.clone();
    tokio::spawn(async move {
        name_owner_watch_on_bus_broadcast(sess, BusType::Session, tx).await;
    });

    rx
}

/// Watch `NameOwnerChanged` on a single bus connection, sending events
/// to a **broadcast** sender. Infinite loop with auto-restart.
async fn name_owner_watch_on_bus_broadcast(
    conn: Connection,
    bus_type: BusType,
    tx: broadcast::Sender<DaemonPresenceEvent>,
) {
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
    }

    loop {
        let proxy = match DBusFdoProxy::new(&conn).await {
            Ok(p) => p,
            Err(e) => {
                warn!(%e, "name watch: failed to create DBus proxy, retrying in 1s");
                tokio::time::sleep(Duration::from_secs(1)).await;
                continue;
            }
        };

        let mut stream = match proxy.receive_name_owner_changed().await {
            Ok(s) => s,
            Err(e) => {
                warn!(%e, "name watch: failed to subscribe to NameOwnerChanged, retrying in 1s");
                tokio::time::sleep(Duration::from_secs(1)).await;
                continue;
            }
        };

        while let Some(msg) = stream.next().await {
            match msg.args() {
                Ok(args) if args.name == DAEMON_BUS_NAME => {
                    if args.new_owner.is_empty() && args.old_owner.is_empty() {
                        continue;
                    }
                    let event = if !args.new_owner.is_empty() {
                        DaemonPresenceEvent::Appeared(bus_type)
                    } else {
                        DaemonPresenceEvent::Disappeared
                    };
                    let _ = tx.send(event);
                }
                Ok(_) => {}
                Err(e) => {
                    warn!(%e, "name watch: failed to parse NameOwnerChanged args");
                }
            }
        }

        warn!("name watch: NameOwnerChanged stream ended, restarting in 1s");
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

// ---------------------------------------------------------------------------
// BusManager — owns dual-bus connections, daemon proxy factory, reconnection
// ---------------------------------------------------------------------------

/// Manages system + session D-Bus connections and the daemon proxy lifecycle.
///
/// Holds connections to BOTH busses simultaneously and uses 4-step resolution
/// to find the daemon (preferring system bus). Provides a factory for creating
/// fresh `DaemonProxy` instances so callers never share a stale proxy.
///
/// All mutable state is behind `Arc<Mutex<…>>` so the manager can be shared
/// across repositories and flows. Lock contention is negligible (a few µs per
/// re-resolution).
#[derive(Debug, Clone)]
pub struct BusManager {
    inner: Arc<Mutex<BusManagerInner>>,
}

#[derive(Debug)]
struct BusManagerInner {
    sys_conn: Connection,
    sess_conn: Connection,
    status: ConnectionStatus,
}

impl BusManager {
    /// Connect to both busses and resolve the daemon (4-step). Falls back to
    /// degraded mode (disconnected) if the daemon is unreachable.
    pub async fn connect() -> Self {
        let sys_conn = Connection::system().await.expect("system bus required");
        let sess_conn = Connection::session().await.unwrap_or_else(|e| {
            warn!(%e, "session bus unavailable, using system bus");
            sys_conn.clone()
        });

        let status = match select_daemon_bus(&sys_conn, &sess_conn).await {
            Some((_, bt)) => {
                info!("daemon found on {bt:?} bus");
                ConnectionStatus::Connected(bt)
            }
            None => {
                warn!("daemon unreachable — running in degraded mode");
                ConnectionStatus::Disconnected
            }
        };

        Self {
            inner: Arc::new(Mutex::new(BusManagerInner {
                sys_conn,
                sess_conn,
                status,
            })),
        }
    }

    /// Create a fresh `DaemonProxy` on the active connection.
    ///
    /// Returns `Ok(proxy)` if the daemon was reachable at last resolution
    /// AND the proxy can be created.  Returns `Err` when the daemon is
    /// unreachable or the connection is broken.
    pub async fn create_proxy(&self) -> Result<DaemonProxy<'static>, anyhow::Error> {
        let inner = self.inner.lock().await;
        if inner.status.is_connected() {
            let conn = if inner.status.bus_type() == Some(BusType::System) {
                &inner.sys_conn
            } else {
                &inner.sess_conn
            };
            match DaemonProxy::new(conn).await {
                Ok(p) => return Ok(p),
                Err(e) => {
                    warn!(%e, "proxy creation failed on active connection, trying fallback");
                    // Try the other bus in case the daemon moved.
                    let fallback = if inner.status.bus_type() == Some(BusType::System) {
                        &inner.sess_conn
                    } else {
                        &inner.sys_conn
                    };
                    if let Ok(p) = DaemonProxy::new(fallback).await {
                        return Ok(p);
                    }
                }
            }
        }
        // Last resort: try both busses from scratch.
        let (conn, bt) = select_daemon_bus(&inner.sys_conn, &inner.sess_conn)
            .await
            .ok_or_else(|| anyhow::anyhow!("daemon unreachable"))?;
        let proxy = DaemonProxy::new(&conn).await?;
        drop(inner);
        let mut inner = self.inner.lock().await;
        inner.status = ConnectionStatus::Connected(bt);
        Ok(proxy)
    }

    /// Try to re-resolve the daemon (e.g. after a `Disappeared` event).
    /// Returns `true` if the daemon became (or remains) reachable.
    pub async fn re_resolve(&self) -> bool {
        let mut inner = self.inner.lock().await;
        match select_daemon_bus(&inner.sys_conn, &inner.sess_conn).await {
            Some((_, bt)) => {
                inner.status = ConnectionStatus::Connected(bt);
                true
            }
            None => {
                inner.status = ConnectionStatus::Disconnected;
                false
            }
        }
    }

    /// Return the current connection status.
    pub async fn status(&self) -> ConnectionStatus {
        self.inner.lock().await.status
    }

    /// Try to get the active connection without blocking (for signal subs).
    ///
    /// Returns `None` if the lock is contended — callers retry on their own
    /// schedule.  The returned connection is a clone; it outlives the lock.
    pub fn try_proxy_conn(&self) -> Option<Connection> {
        match self.inner.try_lock() {
            Ok(inner) => {
                if inner.status.is_connected() {
                    if inner.status.bus_type() == Some(BusType::System) {
                        Some(inner.sys_conn.clone())
                    } else {
                        Some(inner.sess_conn.clone())
                    }
                } else {
                    Some(inner.sys_conn.clone())
                }
            }
            Err(_) => None,
        }
    }

    /// Return the system bus connection (cloned).
    pub fn system_connection(&self) -> Connection {
        self.inner
            .try_lock()
            .expect("BusManager lock")
            .sys_conn
            .clone()
    }

    /// Return the session bus connection (cloned).
    pub fn session_connection(&self) -> Connection {
        self.inner
            .try_lock()
            .expect("BusManager lock")
            .sess_conn
            .clone()
    }
}

/// Which bus the daemon is connected to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BusType {
    System,
    Session,
}

/// Connection status with the daemon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionStatus {
    Connected(BusType),
    Disconnected,
}

impl ConnectionStatus {
    pub fn is_connected(&self) -> bool {
        matches!(self, ConnectionStatus::Connected(_))
    }

    pub fn bus_type(&self) -> Option<BusType> {
        match self {
            ConnectionStatus::Connected(b) => Some(*b),
            ConnectionStatus::Disconnected => None,
        }
    }
}

/// Select which bus hosts `org.wellbeing.v1.Controller` (prefers system).
///
/// Takes already-established connections to both busses and returns the
/// connection and bus type that has the daemon.
pub(crate) async fn select_daemon_bus(
    sys: &Connection,
    sess: &Connection,
) -> Option<(Connection, BusType)> {
    if name_owner_present(sys, DAEMON_BUS_NAME).await {
        debug!("daemon found on system bus");
        return Some((sys.clone(), BusType::System));
    }
    if name_owner_present(sess, DAEMON_BUS_NAME).await {
        debug!("daemon found on session bus");
        return Some((sess.clone(), BusType::Session));
    }
    if start_service_by_name(sys, DAEMON_BUS_NAME).await
        && name_owner_present(sys, DAEMON_BUS_NAME).await
    {
        info!("daemon activated on system bus");
        return Some((sys.clone(), BusType::System));
    }
    if start_service_by_name(sess, DAEMON_BUS_NAME).await
        && name_owner_present(sess, DAEMON_BUS_NAME).await
    {
        info!("daemon activated on session bus");
        return Some((sess.clone(), BusType::Session));
    }
    warn!("daemon unreachable (degraded mode)");
    None
}

async fn name_owner_present(conn: &Connection, name: &str) -> bool {
    use zbus::fdo::DBusProxy;
    use zbus::names::BusName;
    match DBusProxy::new(conn).await {
        Ok(proxy) => {
            let Ok(bus_name) = BusName::try_from(name) else {
                return false;
            };
            proxy.name_has_owner(bus_name).await.unwrap_or(false)
        }
        Err(_) => false,
    }
}

async fn start_service_by_name(conn: &Connection, name: &str) -> bool {
    use zbus::fdo::DBusProxy;
    use zbus::names::WellKnownName;
    match DBusProxy::new(conn).await {
        Ok(proxy) => {
            let Ok(well_known) = WellKnownName::try_from(name) else {
                return false;
            };
            match proxy.start_service_by_name(well_known, 0u32).await {
                Ok(r) => r == 1 || r == 2,
                Err(_) => false,
            }
        }
        Err(_) => false,
    }
}
