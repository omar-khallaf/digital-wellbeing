//! Reports repository — converts DAO tuple types to domain types.
//!
//! Wraps [`daily_usage::reads`] and [`EventDao`] calls, mapping raw database
//! tuples to pre-aggregated domain types like [`AppUsageSummary`] via GROUP BY
//! queries — no per-row, per-date queries remain.

use wellbeing_core::{
    AppClass, AppUsageSummary, Category, CategoryUsageSummary, DateTotal, DayEventRow,
    TitleUsageSummary, Uid,
};

use crate::store::DbPool;
use crate::store::daos::daily_usage::reads;
use crate::store::daos::events::EventDao;

/// Repository for usage-report and event queries.
pub struct ReportsRepo {
    pool: DbPool,
}

impl ReportsRepo {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    /// Get per-app usage aggregated across a date range, sorted by total_millis DESC.
    ///
    /// Returns one row per app — no date grouping.  The SQL query uses the
    /// generated `total_millis` column so `SUM(total_millis)` is a simple
    /// aggregate over the materialised value rather than repeated addition.
    pub async fn get_app_usage_summary(
        &self,
        start_date: &str,
        end_date: &str,
        uid: u32,
    ) -> anyhow::Result<Vec<AppUsageSummary>> {
        let conn = self.pool.client().await?;
        let rows = reads::app_usage_summary(&conn, start_date, end_date, uid).await?;
        Ok(rows
            .into_iter()
            .map(|r| AppUsageSummary {
                app_class: AppClass::new(&r.0).unwrap_or_else(|_| AppClass::new("?").unwrap()),
                total_millis: r.1,
            })
            .collect())
    }

    /// Get per-title usage aggregated across a date range, sorted by total_millis DESC.
    pub async fn get_title_usage_summary(
        &self,
        start_date: &str,
        end_date: &str,
        uid: u32,
    ) -> anyhow::Result<Vec<TitleUsageSummary>> {
        let conn = self.pool.client().await?;
        let rows = reads::title_usage_summary(&conn, start_date, end_date, uid).await?;
        Ok(rows
            .into_iter()
            .map(|r| TitleUsageSummary {
                app_class: AppClass::new(&r.0).unwrap_or_else(|_| AppClass::new("?").unwrap()),
                title: r.1,
                total_millis: r.2,
            })
            .collect())
    }

    /// Get per-category usage aggregated across a date range, sorted by total_millis DESC.
    pub async fn get_category_usage_summary(
        &self,
        start_date: &str,
        end_date: &str,
        uid: u32,
    ) -> anyhow::Result<Vec<CategoryUsageSummary>> {
        let conn = self.pool.client().await?;
        let rows = reads::category_usage_summary(&conn, start_date, end_date, uid).await?;
        Ok(rows
            .into_iter()
            .filter_map(|r| {
                Category::try_from(r.0).ok().map(|c| CategoryUsageSummary {
                    category: c,
                    total_millis: r.1,
                })
            })
            .collect())
    }

    /// Get total usage per date across a range, sorted by date ASC.
    ///
    /// Returns one row per date — pre-aggregated by SQL so the GUI bar chart
    /// doesn't need to flatten per-entry data and sum by date in memory.
    pub async fn get_daily_bar_totals(
        &self,
        start_date: &str,
        end_date: &str,
        uid: u32,
    ) -> anyhow::Result<Vec<DateTotal>> {
        let conn = self.pool.client().await?;
        let rows = reads::daily_bar_totals(&conn, start_date, end_date, uid).await?;
        Ok(rows
            .into_iter()
            .map(|r| DateTotal {
                date: r.0,
                total_millis: r.1,
            })
            .collect())
    }

    /// Get raw events for a user in a time range (returns domain types).
    pub async fn get_day_events(
        &self,
        uid: Uid,
        start_millis: i64,
        end_millis: i64,
    ) -> anyhow::Result<Vec<DayEventRow>> {
        let conn = self.pool.client().await?;
        EventDao::get_day_events(&conn, uid.0 as i32, start_millis, end_millis).await
    }
}
