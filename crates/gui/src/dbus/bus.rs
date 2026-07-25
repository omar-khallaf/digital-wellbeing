//! D-Bus bus selection and daemon presence watching.
//!
//! Implements the 4-step resolution algorithm from 13-deployment-modes.md
//! and NameOwnerChanged watching for rapid daemon restart detection.

use std::time::Duration;

use futures::StreamExt;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};
use wellbeing_core::dbus_constants::DAEMON_BUS_NAME;
use zbus::Connection;

/// Events from NameOwnerChanged watching for the daemon's bus name.
///
/// Replaces the 10-second polling approach — the D-Bus daemon pushes
/// name change signals immediately instead of the GUI polling every 10s.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DaemonPresenceEvent {
    Appeared,
    Disappeared,
}

/// Spawn background tasks on both busses watching `NameOwnerChanged` for
/// `DAEMON_BUS_NAME`. Returns a receiver for presence change events.
///
/// Both busses are watched simultaneously so cross-bus restarts (daemon moves
/// from system to session bus or vice versa) are detected instantly.
pub fn spawn_daemon_name_watch(
    sys_conn: &Connection,
    sess_conn: &Connection,
) -> mpsc::UnboundedReceiver<DaemonPresenceEvent> {
    let (tx, rx) = mpsc::unbounded_channel();

    let tx_sys = tx.clone();
    let sys = sys_conn.clone();
    let sess = sess_conn.clone();
    tokio::spawn(async move {
        name_owner_watch_on_bus(sys, tx_sys).await;
    });
    tokio::spawn(async move {
        name_owner_watch_on_bus(sess, tx).await;
    });

    rx
}

/// Watch `NameOwnerChanged` on a single bus connection. Infinite loop with
/// auto-restart if the stream drops (e.g., bus connection lost temporarily).
async fn name_owner_watch_on_bus(conn: Connection, tx: mpsc::UnboundedSender<DaemonPresenceEvent>) {
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
                    // Both empty → spurious signal, no state change.
                    if args.new_owner.is_empty() && args.old_owner.is_empty() {
                        continue;
                    }
                    // new_owner present → appeared (or reconnected on same bus).
                    // new_owner absent → disappeared.
                    let event = if !args.new_owner.is_empty() {
                        DaemonPresenceEvent::Appeared
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
