//! Reports repository — converts DAO tuple types to domain types.
//!
//! Wraps [`daily_usage::reads`] and [`EventDao`] calls, mapping raw database
//! tuples to domain types like [`DailyUsageByAppEntry`] and [`DailySummary`].

use std::collections::HashMap;

use wellbeing_core::{
    AppClass, DailySummary, DailyUsageByAppEntry, DailyUsageByCategoryEntry,
    DailyUsageByTitleEntry, DailyUsageByTitleSummary, DayEventRow, Uid,
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

    // ── Conversion helpers (private — tuple aliases can't impl From) ──────

    fn app_entry_from(r: reads::UsageAppRow) -> DailyUsageByAppEntry {
        DailyUsageByAppEntry {
            date: r.0,
            user_id: Uid(r.1 as u32),
            app_class: AppClass::new(&r.2).unwrap_or_else(|_| AppClass::new("?").unwrap()),
            total_millis: r.3 + r.4,
        }
    }

    fn title_entry_from(r: reads::UsageTitleRow) -> DailyUsageByTitleEntry {
        DailyUsageByTitleEntry {
            date: r.0,
            user_id: Uid(r.1 as u32),
            app_class: AppClass::new(&r.2).unwrap_or_else(|_| AppClass::new("?").unwrap()),
            title: r.3,
            total_millis: r.4 + r.5,
        }
    }

    fn category_entry_from(r: reads::UsageCategoryRow) -> DailyUsageByCategoryEntry {
        DailyUsageByCategoryEntry {
            date: r.0,
            user_id: Uid(r.1 as u32),
            category_name: r.2,
            total_millis: r.3 + r.4,
        }
    }

    // ── Per-day queries ──────────────────────────────────────────────────

    /// Get per-app usage for a single date.
    pub async fn get_daily_usage(
        &self,
        date: &str,
        uid: u32,
    ) -> anyhow::Result<Vec<DailyUsageByAppEntry>> {
        let mut conn = self.pool.get().await?;
        let rows = reads::daily_usage_by_app(&mut conn, date, uid).await?;
        Ok(rows.into_iter().map(Self::app_entry_from).collect())
    }

    /// Get per-title usage for a single date.
    pub async fn get_daily_usage_by_title(
        &self,
        date: &str,
        uid: u32,
    ) -> anyhow::Result<Vec<DailyUsageByTitleEntry>> {
        let mut conn = self.pool.get().await?;
        let rows = reads::daily_usage_by_title(&mut conn, date, uid).await?;
        Ok(rows.into_iter().map(Self::title_entry_from).collect())
    }

    // ── Date-range queries ───────────────────────────────────────────────

    /// Get per-app usage for a date range, grouped by date into summaries.
    pub async fn get_usage_range(
        &self,
        start_date: &str,
        end_date: &str,
        uid: u32,
    ) -> anyhow::Result<Vec<DailySummary>> {
        let mut conn = self.pool.get().await?;
        let raw = reads::usage_range_by_app(&mut conn, start_date, end_date, uid).await?;
        let entries: Vec<DailyUsageByAppEntry> =
            raw.into_iter().map(Self::app_entry_from).collect();

        // Pure domain-level grouping: group by date.
        let mut grouped: HashMap<String, Vec<DailyUsageByAppEntry>> = HashMap::new();
        for entry in entries {
            grouped.entry(entry.date.clone()).or_default().push(entry);
        }

        let mut summaries: Vec<DailySummary> = grouped
            .into_iter()
            .map(|(date, entries)| DailySummary {
                date,
                user_id: entries
                    .as_slice()
                    .first()
                    .map(|e| e.user_id)
                    .unwrap_or(Uid(uid)),
                entries,
            })
            .collect();

        summaries.sort_by(|a, b| a.date.cmp(&b.date));
        Ok(summaries)
    }

    /// Get per-title usage for a date range, grouped by date into summaries.
    pub async fn get_usage_range_by_title(
        &self,
        start_date: &str,
        end_date: &str,
        uid: u32,
    ) -> anyhow::Result<Vec<DailyUsageByTitleSummary>> {
        let mut conn = self.pool.get().await?;
        let raw = reads::usage_range_by_title(&mut conn, start_date, end_date, uid).await?;
        let entries: Vec<DailyUsageByTitleEntry> =
            raw.into_iter().map(Self::title_entry_from).collect();

        // Pure domain-level grouping: group entries by date.
        let mut summaries: Vec<DailyUsageByTitleSummary> = Vec::new();
        for entry in entries {
            if let Some(s) = summaries.last_mut()
                && s.date == entry.date
            {
                s.entries.push(entry);
                continue;
            }
            let date = entry.date.clone();
            let user_id = entry.user_id;
            summaries.push(DailyUsageByTitleSummary {
                date,
                user_id,
                entries: vec![entry],
            });
        }
        Ok(summaries)
    }

    /// Get per-category usage for a date range.
    pub async fn get_usage_range_by_category(
        &self,
        start_date: &str,
        end_date: &str,
        uid: u32,
    ) -> anyhow::Result<Vec<DailyUsageByCategoryEntry>> {
        let mut conn = self.pool.get().await?;
        let rows = reads::usage_range_by_category(&mut conn, start_date, end_date, uid).await?;
        Ok(rows.into_iter().map(Self::category_entry_from).collect())
    }

    /// Get raw events for a user in a time range (returns domain types).
    pub async fn get_day_events(
        &self,
        uid: Uid,
        start_millis: i64,
        end_millis: i64,
    ) -> anyhow::Result<Vec<DayEventRow>> {
        let mut conn = self.pool.get().await?;
        EventDao::get_day_events(&mut conn, uid.0 as i32, start_millis, end_millis).await
    }
}
