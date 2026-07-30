//! ReportsRepository — timeout-guarded D-Bus fetches with boundary validation.

use std::time::Duration;

use anyhow::Result;
use tokio::time::timeout;
use tracing::warn;
use wellbeing_core::*;

use crate::dbus::BusManager;
use crate::dbus::client::DaemonProxy;

const DBUS_TIMEOUT: Duration = Duration::from_secs(10);

/// Repository for reports screen D-Bus queries.
#[derive(Debug, Clone)]
pub struct ReportsRepo {
    pub(crate) bus: BusManager,
}

impl ReportsRepo {
    pub fn new(bus: BusManager) -> Self {
        Self { bus }
    }

    pub(crate) async fn proxy(&self) -> Result<DaemonProxy<'static>> {
        self.bus.create_proxy().await
    }

    /// Fetch per-date total usage across a range, pre-aggregated by SQL.
    pub async fn get_daily_bar_totals(
        &self,
        start: &str,
        end: &str,
        uid: u32,
    ) -> Result<Vec<DateTotal>> {
        let proxy = self.proxy().await?;
        timeout(DBUS_TIMEOUT, proxy.get_daily_bar_totals(start, end, uid))
            .await
            .map_err(|_| anyhow::anyhow!("timeout: get_daily_bar_totals"))?
            .map_err(Into::into)
    }

    pub async fn get_app_categories(&self) -> Result<Vec<AppCategoryRow>> {
        let proxy = self.proxy().await?;
        timeout(DBUS_TIMEOUT, proxy.get_app_categories())
            .await
            .map_err(|_| anyhow::anyhow!("timeout: get_app_categories"))?
            .map_err(Into::into)
    }

    /// Fetch aggregated per-app totals across a date range, sorted by total_millis DESC.
    pub async fn get_app_usage_summary(
        &self,
        start: &str,
        end: &str,
        uid: u32,
    ) -> Result<Vec<AppUsageSummary>> {
        let proxy = self.proxy().await?;
        timeout(DBUS_TIMEOUT, proxy.get_app_usage_summary(start, end, uid))
            .await
            .map_err(|_| anyhow::anyhow!("timeout: get_app_usage_summary"))?
            .map_err(Into::into)
    }

    /// Fetch aggregated per-title totals across a date range, sorted by total_millis DESC.
    pub async fn get_title_usage_summary(
        &self,
        start: &str,
        end: &str,
        uid: u32,
    ) -> Result<Vec<TitleUsageSummary>> {
        let proxy = self.proxy().await?;
        timeout(DBUS_TIMEOUT, proxy.get_title_usage_summary(start, end, uid))
            .await
            .map_err(|_| anyhow::anyhow!("timeout: get_title_usage_summary"))?
            .map_err(Into::into)
    }

    /// Fetch all data needed for the reports screen.
    ///
    /// All summary fields (app, title, daily bar) arrive pre-aggregated by
    /// SQL — no GUI-side sorting or flattening needed.
    pub async fn fetch_all(&self, uid: u32, range: DateRange) -> Result<ReportsData> {
        let start = range.start_str();
        let end = range.end_str();

        let (daily_totals, app_cats, app_summary, title_summary) = tokio::join!(
            self.get_daily_bar_totals(&start, &end, uid),
            self.get_app_categories(),
            self.get_app_usage_summary(&start, &end, uid),
            self.get_title_usage_summary(&start, &end, uid),
        );

        Ok(ReportsData {
            daily_totals: daily_totals.unwrap_or_else(|e| {
                warn!("reports: get_daily_bar_totals failed: {e}");
                vec![]
            }),
            app_categories: app_cats.unwrap_or_else(|e| {
                warn!("reports: get_app_categories failed: {e}");
                vec![]
            }),
            app_summary: app_summary.unwrap_or_else(|e| {
                warn!("reports: get_app_usage_summary failed: {e}");
                vec![]
            }),
            title_summary: title_summary.unwrap_or_else(|e| {
                warn!("reports: get_title_usage_summary failed: {e}");
                vec![]
            }),
        })
    }
}

/// Raw D-Bus response bundle for the reports screen.
#[derive(Debug, Clone, PartialEq)]
pub struct ReportsData {
    /// Per-date total usage for the bar chart, pre-aggregated by SQL.
    pub daily_totals: Vec<DateTotal>,
    pub app_categories: Vec<AppCategoryRow>,
    /// Pre-aggregated per-app totals, sorted by total_millis DESC from SQL.
    pub app_summary: Vec<AppUsageSummary>,
    /// Pre-aggregated per-title totals, sorted by total_millis DESC from SQL.
    pub title_summary: Vec<TitleUsageSummary>,
}
