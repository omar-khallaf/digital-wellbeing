//! GUI D-Bus client: `DaemonClient` proxy wrapper.
//!
//! Holds connections to BOTH system and session busses simultaneously,
//! using 4-step resolution to prefer the system daemon.

use std::sync::Arc;

use anyhow::{Context, Result};
use wellbeing_core::*;
use zbus::Connection;
use zbus::proxy;

use crate::cache::ClientCache;

use super::bus::{ConnectionStatus, select_daemon_bus};

#[proxy(
    interface = "org.wellbeing.v1.Controller",
    default_service = "org.wellbeing.v1.Controller",
    default_path = "/org/wellbeing/Controller"
)]
pub(crate) trait Daemon {
    async fn list_policies(&self, filter_owner: u32) -> zbus::Result<Vec<PolicyData>>;
    async fn create_policy(&self, input: PolicyInput) -> zbus::Result<PolicyId>;
    async fn update_policy(&self, id: PolicyId, input: PolicyInput) -> zbus::Result<()>;
    async fn delete_policy(&self, id: PolicyId) -> zbus::Result<()>;

    async fn get_daily_usage(
        &self,
        date: &str,
        user_id: u32,
    ) -> zbus::Result<Vec<DailyUsageByAppEntry>>;
    async fn get_usage_range(
        &self,
        start_date: &str,
        end_date: &str,
        user_id: u32,
    ) -> zbus::Result<Vec<DailySummary>>;

    async fn get_daily_usage_by_title(
        &self,
        date: &str,
        user_id: u32,
    ) -> zbus::Result<Vec<DailyUsageByTitleEntry>>;
    async fn get_usage_range_by_title(
        &self,
        start_date: &str,
        end_date: &str,
        user_id: u32,
    ) -> zbus::Result<Vec<DailyUsageByTitleSummary>>;

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
    fn blocked_apps(&self) -> zbus::Result<Vec<BlockedAppEntry>>;

    /// Signals (non-async — zbus generates receivers)
    #[zbus(signal, name = "BlockedAppsChanged")]
    fn on_blocked_apps_changed(&self) -> zbus::Result<(u32, String, bool, u32)>;

    #[zbus(signal)]
    fn daily_usage_changed(&self) -> zbus::Result<u32>;

    #[zbus(signal)]
    fn policy_mutated(&self) -> zbus::Result<u32>;
}

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
    range_by_title_cache: Arc<ClientCache<String, Vec<DailyUsageByTitleSummary>>>,
    daily_title_cache: Arc<ClientCache<String, Vec<DailyUsageByTitleEntry>>>,
    day_events_cache: Arc<ClientCache<String, Vec<DayEventRow>>>,
    policy_cache: Arc<ClientCache<String, Vec<PolicyData>>>,
    category_cache: Arc<ClientCache<String, Vec<Category>>>,
    app_category_cache: Arc<ClientCache<String, Vec<AppCategoryRow>>>,
}

impl DaemonClient {
    /// Select the daemon via 4-step resolution, preferring system bus.
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
            range_by_title_cache: Arc::new(ClientCache::new()),
            daily_title_cache: Arc::new(ClientCache::new()),
            day_events_cache: Arc::new(ClientCache::new()),
            policy_cache: Arc::new(ClientCache::new()),
            category_cache: Arc::new(ClientCache::new()),
            app_category_cache: Arc::new(ClientCache::new()),
        })
    }

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
            range_by_title_cache: Arc::new(ClientCache::new()),
            daily_title_cache: Arc::new(ClientCache::new()),
            day_events_cache: Arc::new(ClientCache::new()),
            policy_cache: Arc::new(ClientCache::new()),
            category_cache: Arc::new(ClientCache::new()),
            app_category_cache: Arc::new(ClientCache::new()),
        }
    }

    pub fn connection_status(&self) -> ConnectionStatus {
        self.status
    }

    /// The underlying zbus connection for the active daemon bus (for signal subscriptions).
    pub fn connection(&self) -> &Connection {
        &self.active_conn
    }

    pub fn system_connection(&self) -> &Connection {
        &self.sys_conn
    }

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

    pub async fn get_daily_usage_by_title(
        &self,
        date: &str,
        user_id: u32,
    ) -> Result<Vec<DailyUsageByTitleEntry>> {
        let key = format!("title:{}:{}", date, user_id);
        if let Some(cached) = self.daily_title_cache.get(&key) {
            return Ok(cached);
        }
        let entries = self.proxy.get_daily_usage_by_title(date, user_id).await?;
        self.daily_title_cache.set(key, entries.clone());
        Ok(entries)
    }

    pub async fn get_usage_range_by_title(
        &self,
        start_date: &str,
        end_date: &str,
        user_id: u32,
    ) -> Result<Vec<DailyUsageByTitleSummary>> {
        let key = format!("range_by_title:{}:{}:{}", start_date, end_date, user_id);
        if let Some(cached) = self.range_by_title_cache.get(&key) {
            return Ok(cached);
        }
        let entries = self
            .proxy
            .get_usage_range_by_title(start_date, end_date, user_id)
            .await?;
        self.range_by_title_cache.set(key, entries.clone());
        Ok(entries)
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

    pub async fn get_blocked_apps(&self) -> Result<Vec<BlockedAppEntry>> {
        self.proxy.blocked_apps().await.map_err(Into::into)
    }

    pub fn invalidate_range_cache(&self) {
        self.range_cache.clear();
    }
    pub fn invalidate_range_by_title_cache(&self) {
        self.range_by_title_cache.clear();
    }
    pub fn invalidate_daily_title_cache(&self) {
        self.daily_title_cache.clear();
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
