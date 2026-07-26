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

    pub async fn get_usage_range_by_title(
        &self,
        start: &str,
        end: &str,
        uid: u32,
    ) -> Result<Vec<DailyUsageByTitleSummary>> {
        let proxy = self.proxy().await?;
        timeout(
            DBUS_TIMEOUT,
            proxy.get_usage_range_by_title(start, end, uid),
        )
        .await
        .map_err(|_| anyhow::anyhow!("timeout: get_usage_range_by_title"))?
        .map_err(Into::into)
    }

    pub async fn get_app_categories(&self) -> Result<Vec<AppCategoryRow>> {
        let proxy = self.proxy().await?;
        timeout(DBUS_TIMEOUT, proxy.get_app_categories())
            .await
            .map_err(|_| anyhow::anyhow!("timeout: get_app_categories"))?
            .map_err(Into::into)
    }

    /// Fetch all data needed to build a `ReportsViewModel`.
    pub async fn fetch_all(&self, uid: u32, range: DateRange) -> Result<ReportsData> {
        let start = range.start_str();
        let end = range.end_str();

        let (usage, title_summaries, app_cats) = tokio::join!(
            self.get_usage_range(&start, &end, uid),
            self.get_usage_range_by_title(&start, &end, uid),
            self.get_app_categories(),
        );

        let title_entries: Vec<DailyUsageByTitleEntry> = title_summaries
            .unwrap_or_else(|e| {
                warn!("reports: get_usage_range_by_title failed: {e}");
                vec![]
            })
            .into_iter()
            .flat_map(|s| s.entries)
            .collect();

        Ok(ReportsData {
            summaries: usage.unwrap_or_else(|e| {
                warn!("reports: get_usage_range failed: {e}");
                vec![]
            }),
            title_entries,
            app_categories: app_cats.unwrap_or_else(|e| {
                warn!("reports: get_app_categories failed: {e}");
                vec![]
            }),
        })
    }
}

/// Raw D-Bus response bundle for the reports screen.
#[derive(Debug, Clone)]
pub struct ReportsData {
    pub summaries: Vec<DailySummary>,
    pub title_entries: Vec<DailyUsageByTitleEntry>,
    pub app_categories: Vec<AppCategoryRow>,
}
