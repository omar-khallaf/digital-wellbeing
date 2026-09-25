//! DashboardRepository — timeout-guarded D-Bus fetches with boundary validation.
//!
//! Each method wraps a zbus proxy call in `tokio::time::timeout` so a hung
//! connection never blocks the caller indefinitely.  Returns domain types
//! directly; the caller (viewmodel builder) never touches D-Bus.

use std::time::Duration;

use anyhow::Result;
use tokio::time::timeout;
use tracing::warn;
use wellbeing_core::*;

use crate::dbus::BusManager;
use crate::dbus::client::DaemonProxy;

const DBUS_TIMEOUT: Duration = Duration::from_secs(10);

/// Repository for dashboard D-Bus queries.
///
/// Owns a reference to `BusManager` for proxy creation.  All methods apply a
/// 10-second timeout — if D-Bus hangs the error propagates without blocking
/// the caller's task.
#[derive(Debug, Clone)]
pub struct DashboardRepo {
    pub(crate) bus: BusManager,
}

impl DashboardRepo {
    pub fn new(bus: BusManager) -> Self {
        Self { bus }
    }

    /// Returns `Err` if the daemon is unreachable.
    pub(crate) async fn proxy(&self) -> Result<DaemonProxy<'static>> {
        self.bus.create_proxy().await
    }

    pub async fn get_blocked_apps(&self) -> Result<Vec<BlockedAppEntry>> {
        let proxy = self.proxy().await?;
        timeout(DBUS_TIMEOUT, proxy.get_blocked_apps())
            .await
            .map_err(|_| anyhow::anyhow!("timeout: get_blocked_apps"))?
            .map_err(Into::into)
    }

    pub async fn get_day_events(&self, start_ms: i64, end_ms: i64) -> Result<Vec<DayEventRow>> {
        let proxy = self.proxy().await?;
        timeout(DBUS_TIMEOUT, proxy.get_day_events(start_ms, end_ms))
            .await
            .map_err(|_| anyhow::anyhow!("timeout: get_day_events"))?
            .map_err(Into::into)
    }

    /// Fetch the trailing pre-midnight seed: the last Focus strictly before
    /// `day_start_ms` with no close/Idle after it, trusted regardless of age.
    ///
    /// Walks history backward over `[0, day_start_ms)`, skipping
    /// measurement-ignored Resume rows. Returns `None` when history is empty
    /// or the latest meaningful row already ended the interval (Idle/close).
    pub async fn get_pre_midnight_seed(
        &self,
        day_start_ms: i64,
    ) -> Result<Option<(i64, AppClass)>> {
        let rows = self.get_day_events(0, day_start_ms).await?;
        for row in rows.iter().rev() {
            match row.event_type {
                EventType::Resume => continue,
                EventType::Focus => {
                    return Ok(Some((row.timestamp, row.app_class.clone())));
                }
                EventType::Unfocus
                | EventType::Idle
                | EventType::Suspend
                | EventType::ShutDown
                | EventType::Locked
                | EventType::LoggedOut
                | EventType::Block => return Ok(None),
            }
        }
        Ok(None)
    }

    pub async fn list_categories(&self) -> Result<Vec<Category>> {
        let proxy = self.proxy().await?;
        timeout(DBUS_TIMEOUT, proxy.list_categories())
            .await
            .map_err(|_| anyhow::anyhow!("timeout: list_categories"))?
            .map_err(Into::into)
    }

    pub async fn get_app_categories(&self) -> Result<Vec<AppCategoryRow>> {
        let proxy = self.proxy().await?;
        timeout(DBUS_TIMEOUT, proxy.get_app_categories())
            .await
            .map_err(|_| anyhow::anyhow!("timeout: get_app_categories"))?
            .map_err(Into::into)
    }

    /// Fetch pre-aggregated per-category totals across a date range,
    /// sorted by total_millis DESC from SQL.
    async fn get_category_usage_summary(
        &self,
        start: &str,
        end: &str,
    ) -> Result<Vec<CategoryUsageSummary>> {
        let proxy = self.proxy().await?;
        timeout(DBUS_TIMEOUT, proxy.get_category_usage_summary(start, end))
            .await
            .map_err(|_| anyhow::anyhow!("timeout: get_category_usage_summary"))?
            .map_err(Into::into)
    }

    /// Fetch all data needed to build a `DashboardViewModel`.
    ///
    /// Each D-Bus call has an independent 10s timeout so one hung call
    /// degrades gracefully — the caller sees a partial-error log and can
    /// decide to retry on the next tick.
    ///
    /// App/title/category summary fields arrive pre-sorted by total_millis DESC
    /// (GROUP BY + ORDER BY in SQL via the generated `total_millis`
    /// column) — no GUI-side sorting or aggregation needed.
    pub async fn fetch_all(&self, range: DateRange) -> Result<DashboardData> {
        let start = range.start_str();
        let end = range.end_str();

        let today = chrono::Utc::now().date_naive();
        let day_start_ms = today
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp_millis();
        let day_end_ms = (today + chrono::Days::new(1))
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp_millis();

        let (blocks, day_events, day_seed, cat_summary, cats, app_cats, app_sum, title_sum) = tokio::join!(
            self.get_blocked_apps(),
            self.get_day_events(day_start_ms, day_end_ms),
            self.get_pre_midnight_seed(day_start_ms),
            self.get_category_usage_summary(&start, &end),
            self.list_categories(),
            self.get_app_categories(),
            self.get_app_usage_summary(&start, &end),
            self.get_title_usage_summary(&start, &end),
        );

        Ok(DashboardData {
            blocked: blocks.unwrap_or_else(|e| {
                warn!("dashboard: get_blocked_apps failed: {e}");
                vec![]
            }),
            day_events: day_events.unwrap_or_else(|e| {
                warn!("dashboard: get_day_events failed: {e}");
                vec![]
            }),
            day_seed: day_seed.unwrap_or_else(|e| {
                warn!("dashboard: get_pre_midnight_seed failed: {e}");
                None
            }),
            category_summary: cat_summary.unwrap_or_else(|e| {
                warn!("dashboard: get_category_usage_summary failed: {e}");
                vec![]
            }),
            categories: cats.unwrap_or_else(|e| {
                warn!("dashboard: list_categories failed: {e}");
                vec![]
            }),
            app_categories: app_cats.unwrap_or_else(|e| {
                warn!("dashboard: get_app_categories failed: {e}");
                vec![]
            }),
            app_summary: app_sum.unwrap_or_else(|e| {
                warn!("dashboard: get_app_usage_summary failed: {e}");
                vec![]
            }),
            title_summary: title_sum.unwrap_or_else(|e| {
                warn!("dashboard: get_title_usage_summary failed: {e}");
                vec![]
            }),
        })
    }

    /// Fetch aggregated per-app totals across a date range, sorted by total_millis DESC.
    async fn get_app_usage_summary(&self, start: &str, end: &str) -> Result<Vec<AppUsageSummary>> {
        let proxy = self.proxy().await?;
        timeout(DBUS_TIMEOUT, proxy.get_app_usage_summary(start, end))
            .await
            .map_err(|_| anyhow::anyhow!("timeout: get_app_usage_summary"))?
            .map_err(Into::into)
    }

    /// Fetch aggregated per-title totals across a date range, sorted by total_millis DESC.
    async fn get_title_usage_summary(
        &self,
        start: &str,
        end: &str,
    ) -> Result<Vec<TitleUsageSummary>> {
        let proxy = self.proxy().await?;
        timeout(DBUS_TIMEOUT, proxy.get_title_usage_summary(start, end))
            .await
            .map_err(|_| anyhow::anyhow!("timeout: get_title_usage_summary"))?
            .map_err(Into::into)
    }
}

/// Raw D-Bus response bundle for the dashboard screen.
#[derive(Debug, Clone, PartialEq)]
pub struct DashboardData {
    pub blocked: Vec<BlockedAppEntry>,
    pub day_events: Vec<DayEventRow>,
    /// Trailing pre-midnight seed: last pre-midnight Focus `(timestamp, app)`
    /// with no close/Idle after it, or `None` when the interval already ended.
    pub day_seed: Option<(i64, AppClass)>,
    /// Pre-aggregated per-category totals, sorted by total_millis DESC from SQL.
    pub category_summary: Vec<CategoryUsageSummary>,
    pub categories: Vec<Category>,
    pub app_categories: Vec<AppCategoryRow>,
    /// Pre-aggregated per-app totals, sorted by total_millis DESC from SQL.
    pub app_summary: Vec<AppUsageSummary>,
    /// Pre-aggregated per-title totals, sorted by total_millis DESC from SQL.
    pub title_summary: Vec<TitleUsageSummary>,
}
