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

    /// Create a fresh proxy.  Returns `Err` if the daemon is unreachable.
    pub(crate) async fn proxy(&self) -> Result<DaemonProxy<'static>> {
        self.bus.create_proxy().await
    }

    pub async fn get_usage_range(
        &self,
        start: &str,
        end: &str,
        uid: u32,
    ) -> Result<Vec<DailySummary>> {
        let proxy = self.proxy().await?;
        timeout(DBUS_TIMEOUT, proxy.get_usage_range(start, end, uid))
            .await
            .map_err(|_| anyhow::anyhow!("timeout: get_usage_range"))?
            .map_err(Into::into)
    }

    pub async fn get_blocked_apps(&self) -> Result<Vec<BlockedAppEntry>> {
        let proxy = self.proxy().await?;
        timeout(DBUS_TIMEOUT, proxy.blocked_apps())
            .await
            .map_err(|_| anyhow::anyhow!("timeout: blocked_apps"))?
            .map_err(Into::into)
    }

    pub async fn get_day_events(
        &self,
        uid: u32,
        start_ms: i64,
        end_ms: i64,
    ) -> Result<Vec<DayEventRow>> {
        let proxy = self.proxy().await?;
        timeout(DBUS_TIMEOUT, proxy.get_day_events(uid, start_ms, end_ms))
            .await
            .map_err(|_| anyhow::anyhow!("timeout: get_day_events"))?
            .map_err(Into::into)
    }

    pub async fn get_daily_usage_by_title(
        &self,
        date: &str,
        uid: u32,
    ) -> Result<Vec<DailyUsageByTitleEntry>> {
        let proxy = self.proxy().await?;
        timeout(DBUS_TIMEOUT, proxy.get_daily_usage_by_title(date, uid))
            .await
            .map_err(|_| anyhow::anyhow!("timeout: get_daily_usage_by_title"))?
            .map_err(Into::into)
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

    /// Fetch all data needed to build a `DashboardViewModel`.
    ///
    /// Each D-Bus call has an independent 10s timeout so one hung call
    /// degrades gracefully — the caller sees a partial-error log and can
    /// decide to retry on the next tick.
    pub async fn fetch_all(&self, uid: u32, range: DateRange) -> Result<DashboardData> {
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

        let (usage, blocks, day_events, title, cats, app_cats) = tokio::join!(
            self.get_usage_range(&start, &end, uid),
            self.get_blocked_apps(),
            self.get_day_events(uid, day_start_ms, day_end_ms),
            self.get_daily_usage_by_title(&start, uid),
            self.list_categories(),
            self.get_app_categories(),
        );

        Ok(DashboardData {
            summaries: usage.unwrap_or_else(|e| {
                warn!("dashboard: get_usage_range failed: {e}");
                vec![]
            }),
            blocked: blocks.unwrap_or_else(|e| {
                warn!("dashboard: blocked_apps failed: {e}");
                vec![]
            }),
            day_events: day_events.unwrap_or_else(|e| {
                warn!("dashboard: get_day_events failed: {e}");
                vec![]
            }),
            title_entries: title.unwrap_or_else(|e| {
                warn!("dashboard: get_daily_usage_by_title failed: {e}");
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
        })
    }
}

/// Raw D-Bus response bundle for the dashboard screen.
#[derive(Debug, Clone)]
pub struct DashboardData {
    pub summaries: Vec<DailySummary>,
    pub blocked: Vec<BlockedAppEntry>,
    pub day_events: Vec<DayEventRow>,
    pub title_entries: Vec<DailyUsageByTitleEntry>,
    pub categories: Vec<Category>,
    pub app_categories: Vec<AppCategoryRow>,
}
