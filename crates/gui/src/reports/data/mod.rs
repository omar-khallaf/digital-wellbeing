//! Reports data layer — ViewModel builder, repository, and background flow.

mod flow;
mod repo;

pub use flow::{FlowState, spawn_reports_flow};
pub use repo::{ReportsData, ReportsRepo};

use std::collections::{BTreeMap, HashMap};

use chrono::{NaiveDate, Utc};
use wellbeing_core::DateRange;

use crate::chart::HasBarData;

use super::domain::{DailyBar, ReportAppEntry, ReportTitleEntry, ReportsViewModel};

// ---------------------------------------------------------------------------
// ViewModel mutation method (Compose-like StateFlow pattern)
// ---------------------------------------------------------------------------

impl ReportsViewModel {
    /// Recompute ALL derived fields from the raw data in `self.data`.
    ///
    /// Call after a full fetch (DailyUsageChanged / daemon reconnect / manual
    /// refresh).  The `range` parameter comes from the user's selected date
    /// range (unlike the dashboard which always shows today).
    ///
    /// App and title lists now use the pre-aggregated, pre-sorted
    /// `app_summary` / `title_summary` fields from the daemon (GROUP BY +
    /// ORDER BY total_millis DESC in SQL via the generated `total_millis`
    /// column) — no GUI-side sorting or aggregation needed.
    pub fn recompute_derived(&mut self, range: DateRange) {
        let Some(ref data) = self.data else {
            self.bar_chart.clear();
            self.app_list.clear();
            self.title_list.clear();
            self.total_millis = 0;
            self.top_app = String::new();
            return;
        };

        // ── Bar chart: pre-aggregated per-date totals from SQL ────────────────
        let mut by_date: BTreeMap<String, f64> = BTreeMap::new();
        let mut total: f64 = 0.0;
        for dt in &data.daily_totals {
            let day_ms = dt.total_millis as f64;
            by_date.insert(dt.date.clone(), day_ms);
            total += day_ms;
        }

        let today = Utc::now().date_naive();
        let day_count = (range.end - range.start).num_days() as usize + 1;
        let mut bar_chart: Vec<DailyBar> = Vec::with_capacity(day_count);
        let mut cursor = range.start;
        while cursor <= range.end {
            let key = cursor.format("%Y-%m-%d").to_string();
            let millis = by_date.get(&key).copied().unwrap_or(0.0);
            bar_chart.push(DailyBar {
                date: cursor,
                hours_tracked: millis / 3_600_000.0,
                is_today: cursor == today,
            });
            cursor = cursor + chrono::Days::new(1);
        }

        // ── App list (pre-sorted by total_millis DESC from SQL) ───────────────
        let meta: HashMap<&str, &str> = data
            .app_categories
            .iter()
            .map(|ac| (ac.app_class.as_str(), ac.display_name.as_str()))
            .collect();

        let total_millis_all = data.app_summary.iter().map(|a| a.total_millis).sum::<i64>() as f64;
        let mut app_list: Vec<ReportAppEntry> = data
            .app_summary
            .iter()
            .map(|s| {
                let app_class = s.app_class.as_ref().to_string();
                let display_name = meta
                    .get(app_class.as_str())
                    .copied()
                    .unwrap_or(&app_class)
                    .to_string();
                ReportAppEntry {
                    rank: 0,
                    total_millis: s.total_millis,
                    percentage: if total_millis_all > 0.0 {
                        (s.total_millis as f64 / total_millis_all) * 100.0
                    } else {
                        0.0
                    },
                    app_class,
                    display_name,
                }
            })
            .collect();

        // Data is already sorted by SQL, just assign ranks.
        for (i, entry) in app_list.iter_mut().enumerate() {
            entry.rank = i + 1;
        }

        // ── Title list (pre-sorted by total_millis DESC from SQL) ─────────────
        let title_total: f64 = data
            .title_summary
            .iter()
            .map(|t| t.total_millis)
            .sum::<i64>() as f64;
        let mut title_list: Vec<ReportTitleEntry> = data
            .title_summary
            .iter()
            .map(|s| {
                let app_class = s.app_class.as_ref().to_string();
                let display_name = meta
                    .get(app_class.as_str())
                    .copied()
                    .unwrap_or(&app_class)
                    .to_string();
                ReportTitleEntry {
                    rank: 0,
                    app_class: display_name,
                    title: s.title.clone(),
                    total_millis: s.total_millis,
                    percentage: if title_total > 0.0 {
                        (s.total_millis as f64 / title_total) * 100.0
                    } else {
                        0.0
                    },
                }
            })
            .collect();

        // Data is already sorted by SQL, just assign ranks.
        for (i, entry) in title_list.iter_mut().enumerate() {
            entry.rank = i + 1;
        }

        // ── Top-level KPIs ────────────────────────────────────────────────────
        let top_app = app_list
            .first()
            .map(|s| s.display_name.clone())
            .unwrap_or_else(|| "\u{2014}".into());

        self.bar_chart = bar_chart;
        self.app_list = app_list;
        self.title_list = title_list;
        self.total_millis = total as i64;
        self.top_app = top_app;
        self.date_range = range;
    }
}

// ---------------------------------------------------------------------------
// HasBarData for DailyBar (shared chart trait)
// ---------------------------------------------------------------------------

impl HasBarData for DailyBar {
    fn date(&self) -> NaiveDate {
        self.date
    }
    fn total_millis(&self) -> f64 {
        self.hours_tracked * 3_600_000.0
    }
}
