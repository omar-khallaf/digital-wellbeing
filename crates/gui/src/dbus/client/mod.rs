//! `DaemonClient` — thin wrapper around the daemon's D-Bus API.
//!
//! Holds connections to BOTH system and session busses simultaneously,
//! using 4-step resolution to prefer the system daemon. Provides response
//! caching with invalidation methods.

use std::sync::Arc;

use anyhow::{Context, Result};
use wellbeing_core::*;
use zbus::Connection;

use crate::cache::ClientCache;

use super::bus::{ConnectionStatus, select_daemon_bus};

mod methods;
mod proxy;

pub(crate) use proxy::DaemonProxy;

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

    // -- Cache invalidation methods --

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
