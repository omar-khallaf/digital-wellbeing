//! GUI D-Bus client: `DaemonClient` proxy + `SignalCoalescer` + bus selection.
//!
//! The GUI never touches SQLite — all data flows through the daemon's D-Bus API
//! (`org.wellbeing.v1.Daemon`). Responses are cached via `ClientCache` to
//! deduplicate repeated D-Bus calls within a single refresh cycle; callers
//! explicitly invalidate the cache when signals indicate data has changed.
//!
//! ## Architecture (06-daemon-dbus.md / 09-state-flow.md)
//!
//! - `DaemonClient` wraps a zbus proxy and per-method `ClientCache` instances.
//! - `SignalCoalescer` converts bursty D-Bus signals into coalesced notifications.
//! - `select_daemon_bus()` implements the 4-step selection from 13-deployment-modes.md.
//! - The client holds connections to BOTH system and session busses simultaneously,
//!   using 4-step resolution to prefer the system daemon. This enables recovery
//!   from daemon restarts even if the daemon reappears on a different bus.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{Context, Result};
use futures::StreamExt;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};
use wellbeing_core::dbus_constants::DAEMON_BUS_NAME;
use wellbeing_core::*;
use zbus::Connection;
use zbus::proxy;

use crate::cache::ClientCache;

// ── Daemon Presence Watch (NameOwnerChanged) ────────────────────────────────

/// Events from NameOwnerChanged watching for the daemon's bus name.
///
/// Replaces the 10-second polling approach — the D-Bus daemon pushes
/// name change signals immediately instead of the GUI polling every 10s.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DaemonPresenceEvent {
    /// Daemon appeared on a bus (new_owner present).
    Appeared,
    /// Daemon disappeared from a bus (new_owner absent).
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

// ── Connection status types ──────────────────────────────────────────────────

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

// ── D-Bus proxy trait (generated by zbus compile-time macro) ────────────────

#[proxy(
    interface = "org.wellbeing.v1.Controller",
    default_service = "org.wellbeing.v1.Controller",
    default_path = "/org/wellbeing/Controller"
)]
trait Daemon {
    async fn list_policies(&self, filter_owner: u32) -> zbus::Result<Vec<PolicyData>>;
    async fn create_policy(&self, input: PolicyInput) -> zbus::Result<PolicyId>;
    async fn update_policy(&self, id: PolicyId, input: PolicyInput) -> zbus::Result<()>;
    async fn delete_policy(&self, id: PolicyId) -> zbus::Result<()>;

    async fn get_daily_usage(&self, date: &str, user_id: u32)
    -> zbus::Result<Vec<DailyUsageEntry>>;
    async fn get_usage_range(
        &self,
        start_date: &str,
        end_date: &str,
        user_id: u32,
    ) -> zbus::Result<Vec<DailySummary>>;

    async fn get_day_events(
        &self,
        uid: u32,
        start_millis: i64,
        end_millis: i64,
    ) -> zbus::Result<Vec<DayEventRow>>;

    async fn list_categories(&self) -> zbus::Result<Vec<Category>>;
    async fn get_app_categories(&self) -> zbus::Result<Vec<AppCategoryRow>>;
    async fn set_app_category(&self, app_id: &str, category_id: CategoryId) -> zbus::Result<()>;

    #[zbus(property)]
    fn active_blocks(&self) -> zbus::Result<Vec<ActiveBlockEntry>>;

    /// Signals (non-async — zbus generates receivers)
    #[zbus(signal)]
    fn block_state_changed(&self) -> zbus::Result<(u32, String, bool, u32)>;

    #[zbus(signal)]
    fn daily_usage_changed(&self) -> zbus::Result<u32>;

    #[zbus(signal)]
    fn policy_mutated(&self) -> zbus::Result<u32>;
}

// ── Signal Coalescer ────────────────────────────────────────────────────────

/// Coalesces rapid-fire D-Bus signals into periodic cache invalidations.
#[derive(Debug)]
pub struct SignalCoalescer {
    blocked_dirty: AtomicBool,
    usage_dirty: AtomicBool,
    policy_dirty: AtomicBool,
}

/// Bitmask of dirty flags returned by `SignalCoalescer::drain()`.
#[derive(Debug, Clone, Copy, Default)]
pub struct CoalescedNotifications {
    pub blocked: bool,
    pub usage: bool,
    pub policy: bool,
}

impl CoalescedNotifications {
    pub fn any(&self) -> bool {
        self.blocked || self.usage || self.policy
    }
}

impl SignalCoalescer {
    pub fn new() -> Self {
        Self {
            blocked_dirty: AtomicBool::new(false),
            usage_dirty: AtomicBool::new(false),
            policy_dirty: AtomicBool::new(false),
        }
    }

    pub fn mark_blocked_dirty(&self) {
        self.blocked_dirty.store(true, Ordering::Release);
    }

    pub fn mark_daily_usage_dirty(&self) {
        self.usage_dirty.store(true, Ordering::Release);
    }

    pub fn mark_policy_dirty(&self) {
        self.policy_dirty.store(true, Ordering::Release);
    }

    pub fn drain(&self) -> CoalescedNotifications {
        CoalescedNotifications {
            blocked: self.blocked_dirty.swap(false, Ordering::AcqRel),
            usage: self.usage_dirty.swap(false, Ordering::AcqRel),
            policy: self.policy_dirty.swap(false, Ordering::AcqRel),
        }
    }
}

impl Default for SignalCoalescer {
    fn default() -> Self {
        Self::new()
    }
}

// ── DaemonClient ────────────────────────────────────────────────────────────

/// Thin wrapper around the daemon's `org.wellbeing.v1.Controller` D-Bus API.
///
/// Holds connections to BOTH system and session busses simultaneously.
/// Uses 4-step resolution to select which connection hosts the daemon
/// (preferring system), enabling cross-bus daemon restart recovery.
#[derive(Debug, Clone)]
pub struct DaemonClient {
    proxy: DaemonProxy<'static>,
    sys_conn: Connection,
    sess_conn: Connection,
    active_conn: Connection,
    status: ConnectionStatus,
    range_cache: Arc<ClientCache<String, Vec<DailySummary>>>,
    day_events_cache: Arc<ClientCache<String, Vec<DayEventRow>>>,
    policy_cache: Arc<ClientCache<String, Vec<PolicyData>>>,
    category_cache: Arc<ClientCache<String, Vec<Category>>>,
    app_category_cache: Arc<ClientCache<String, Vec<AppCategoryRow>>>,
}

impl DaemonClient {
    /// Connect to BOTH busses and select the daemon via 4-step resolution.
    ///
    /// Prefers system bus; falls back to session if system is unreachable.
    pub async fn connect() -> Result<Self> {
        let sys_conn = Connection::system()
            .await
            .context("failed to connect to system bus")?;
        let sess_conn = match Connection::session().await {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(%e, "session bus unavailable, using system bus as fallback");
                sys_conn.clone()
            }
        };

        let (active_conn, bus_type) = select_daemon_bus(&sys_conn, &sess_conn)
            .await
            .context("wellbeing daemon unreachable (all 4 bus resolution steps failed)")?;

        let proxy = DaemonProxy::new(&active_conn)
            .await
            .context("failed to create daemon D-Bus proxy")?;

        Ok(Self {
            proxy,
            status: ConnectionStatus::Connected(bus_type),
            sys_conn,
            sess_conn,
            active_conn,
            range_cache: Arc::new(ClientCache::new()),
            day_events_cache: Arc::new(ClientCache::new()),
            policy_cache: Arc::new(ClientCache::new()),
            category_cache: Arc::new(ClientCache::new()),
            app_category_cache: Arc::new(ClientCache::new()),
        })
    }

    /// Create a degraded client when the daemon is unreachable.
    ///
    /// Still connects to available busses for signal readiness; sets status to
    /// Disconnected. Periodic re-resolution can pick up the daemon later.
    pub async fn degraded() -> Self {
        let sys_conn = Connection::system()
            .await
            .expect("system bus required for degraded mode");
        let sess_conn = match Connection::session().await {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(%e, "session bus unavailable in degraded mode, using system bus as fallback");
                sys_conn.clone()
            }
        };
        // In degraded mode use session connection as active (for signal
        // subscription, even though daemon isn't there).
        let active_conn = sess_conn.clone();
        let proxy = DaemonProxy::new(&active_conn)
            .await
            .expect("failed to create degraded daemon proxy");
        Self {
            proxy,
            status: ConnectionStatus::Disconnected,
            sys_conn,
            sess_conn,
            active_conn,
            range_cache: Arc::new(ClientCache::new()),
            day_events_cache: Arc::new(ClientCache::new()),
            policy_cache: Arc::new(ClientCache::new()),
            category_cache: Arc::new(ClientCache::new()),
            app_category_cache: Arc::new(ClientCache::new()),
        }
    }

    /// The current connection status.
    pub fn connection_status(&self) -> ConnectionStatus {
        self.status
    }

    /// The underlying zbus connection for the active daemon bus (for signal subscriptions).
    pub fn connection(&self) -> &Connection {
        &self.active_conn
    }

    /// The system bus connection.
    pub fn system_connection(&self) -> &Connection {
        &self.sys_conn
    }

    /// The session bus connection.
    pub fn session_connection(&self) -> &Connection {
        &self.sess_conn
    }

    /// Re-resolve the daemon bus and update the active connection.
    ///
    /// Call periodically or after a D-Bus signal suggests the daemon may have
    /// restarted on a different bus. Returns true if the active bus changed.
    pub async fn re_resolve_bus(&mut self) -> bool {
        let result = select_daemon_bus(&self.sys_conn, &self.sess_conn).await;
        match result {
            Some((conn, bt)) => {
                let changed = !matches!(self.status, ConnectionStatus::Connected(b) if b == bt);
                // Re-create proxy on the (possibly new) active connection first.
                // Only update status + active_conn if the proxy succeeds — a dead
                // socket cannot be silently reused.
                let proxy = DaemonProxy::new(&conn).await;
                if let Ok(proxy) = proxy {
                    self.active_conn = conn;
                    self.status = ConnectionStatus::Connected(bt);
                    self.proxy = proxy;
                }
                changed
            }
            None => {
                self.status = ConnectionStatus::Disconnected;
                false
            }
        }
    }

    pub async fn list_policies(&self, filter_owner: u32) -> Result<Vec<PolicyData>> {
        let key = format!("policies:{}", filter_owner);
        if let Some(cached) = self.policy_cache.get(&key) {
            return Ok(cached);
        }
        let policies = self.proxy.list_policies(filter_owner).await?;
        self.policy_cache.set(key, policies.clone());
        Ok(policies)
    }

    pub async fn create_policy(&self, input: PolicyInput) -> Result<PolicyId> {
        let id = self.proxy.create_policy(input).await?;
        self.policy_cache.clear();
        Ok(id)
    }

    pub async fn update_policy(&self, id: PolicyId, input: PolicyInput) -> Result<()> {
        self.proxy.update_policy(id, input).await?;
        self.policy_cache.clear();
        Ok(())
    }

    pub async fn delete_policy(&self, id: PolicyId) -> Result<()> {
        self.proxy.delete_policy(id).await?;
        self.policy_cache.clear();
        Ok(())
    }

    pub async fn get_usage_range(
        &self,
        start_date: &str,
        end_date: &str,
        user_id: u32,
    ) -> Result<Vec<DailySummary>> {
        let key = format!("range:{}:{}:{}", start_date, end_date, user_id);
        if let Some(cached) = self.range_cache.get(&key) {
            return Ok(cached);
        }
        let summaries = self
            .proxy
            .get_usage_range(start_date, end_date, user_id)
            .await?;
        self.range_cache.set(key, summaries.clone());
        Ok(summaries)
    }

    pub async fn get_day_events(
        &self,
        uid: u32,
        start_millis: i64,
        end_millis: i64,
    ) -> Result<Vec<DayEventRow>> {
        let key = format!("day_events:{}:{}:{}", uid, start_millis, end_millis);
        if let Some(cached) = self.day_events_cache.get(&key) {
            return Ok(cached);
        }
        let events = self
            .proxy
            .get_day_events(uid, start_millis, end_millis)
            .await?;
        self.day_events_cache.set(key, events.clone());
        Ok(events)
    }

    pub async fn list_categories(&self) -> Result<Vec<Category>> {
        let key = "categories".into();
        if let Some(cached) = self.category_cache.get(&key) {
            return Ok(cached);
        }
        let cats = self.proxy.list_categories().await?;
        self.category_cache.set(key, cats.clone());
        Ok(cats)
    }

    pub async fn get_app_categories(&self) -> Result<Vec<AppCategoryRow>> {
        let key = "app_categories".into();
        if let Some(cached) = self.app_category_cache.get(&key) {
            return Ok(cached);
        }
        let rows = self.proxy.get_app_categories().await?;
        self.app_category_cache.set(key, rows.clone());
        Ok(rows)
    }

    pub async fn get_active_blocks(&self) -> Result<Vec<ActiveBlockEntry>> {
        self.proxy.active_blocks().await.map_err(Into::into)
    }

    pub fn invalidate_range_cache(&self) {
        self.range_cache.clear();
    }
    pub fn invalidate_day_events_cache(&self) {
        self.day_events_cache.clear();
    }
    pub fn invalidate_policy_cache(&self) {
        self.policy_cache.clear();
    }
    pub fn invalidate_category_caches(&self) {
        self.category_cache.clear();
        self.app_category_cache.clear();
    }
}

// ── Bus Selection (4-step algorithm from 13-deployment-modes.md) ──────────

/// Select which bus hosts `org.wellbeing.v1.Controller` (prefers system).
///
/// Takes already-established connections to both busses and returns the
/// connection and bus type that has the daemon.
async fn select_daemon_bus(sys: &Connection, sess: &Connection) -> Option<(Connection, BusType)> {
    // 1. System bus already has the daemon?
    if name_owner_present(sys, DAEMON_BUS_NAME).await {
        debug!("daemon found on system bus");
        return Some((sys.clone(), BusType::System));
    }
    // 2. Session bus already has the daemon?
    if name_owner_present(sess, DAEMON_BUS_NAME).await {
        debug!("daemon found on session bus");
        return Some((sess.clone(), BusType::Session));
    }
    // 3. Activate the SYSTEM daemon.
    if start_service_by_name(sys, DAEMON_BUS_NAME).await
        && name_owner_present(sys, DAEMON_BUS_NAME).await
    {
        info!("daemon activated on system bus");
        return Some((sys.clone(), BusType::System));
    }
    // 4. Activate the SESSION daemon.
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

// ── Signal Subscription ─────────────────────────────────────────────────────

/// Subscribe to daemon signals and forward coalesced notifications to the GPUI thread.
pub fn spawn_signal_listener(
    client: &DaemonClient,
    coalescer: Arc<SignalCoalescer>,
    signal_tx: mpsc::UnboundedSender<CoalescedNotifications>,
) {
    let conn = client.connection().clone();
    tokio::spawn(async move {
        let proxy = match DaemonProxy::new(&conn).await {
            Ok(p) => p,
            Err(e) => {
                warn!(%e, "failed signal proxy");
                return;
            }
        };

        let tx = signal_tx.clone();
        let coal = coalescer.clone();
        if let Ok(mut stream) = proxy.receive_block_state_changed().await {
            tokio::spawn(async move {
                while let Some(msg) = stream.next().await {
                    if msg
                        .message()
                        .body()
                        .deserialize::<(u32, String, bool, u32)>()
                        .is_ok()
                    {
                        coal.mark_blocked_dirty();
                        let _ = tx.send(CoalescedNotifications {
                            blocked: true,
                            ..Default::default()
                        });
                    }
                }
            });
        }

        let tx = signal_tx.clone();
        let coal = coalescer.clone();
        if let Ok(mut stream) = proxy.receive_daily_usage_changed().await {
            tokio::spawn(async move {
                while let Some(msg) = stream.next().await {
                    if msg.message().body().deserialize::<u32>().is_ok() {
                        coal.mark_daily_usage_dirty();
                        let _ = tx.send(CoalescedNotifications {
                            usage: true,
                            ..Default::default()
                        });
                    }
                }
            });
        }

        let coal = coalescer.clone();
        if let Ok(mut stream) = proxy.receive_policy_mutated().await {
            tokio::spawn(async move {
                while let Some(msg) = stream.next().await {
                    if msg.message().body().deserialize::<u32>().is_ok() {
                        coal.mark_policy_dirty();
                        let _ = signal_tx.send(CoalescedNotifications {
                            policy: true,
                            ..Default::default()
                        });
                    }
                }
            });
        }
    });
}
