//! Reports data layer — ViewModel builder, repository, and background flow.

mod flow;
mod repo;

pub use flow::{FlowState, spawn_reports_flow};
pub use repo::{ReportsData, ReportsRepo};

use std::collections::{BTreeMap, HashMap};

use chrono::{NaiveDate, Utc};
use wellbeing_core::{DailyUsageByAppEntry, DateRange};

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
    pub fn recompute_derived(&mut self, range: DateRange) {
        let Some(ref data) = self.data else {
            self.bar_chart.clear();
            self.app_list.clear();
            self.title_list.clear();
            self.total_millis = 0;
            self.top_app = String::new();
            return;
        };

        let usage: Vec<DailyUsageByAppEntry> = data
            .summaries
            .iter()
            .flat_map(|s| s.entries.iter().cloned())
            .collect();

        // ── Bar chart: aggregate by date ──────────────────────────────────────
        let mut by_date: BTreeMap<String, f64> = BTreeMap::new();
        let mut by_app: BTreeMap<String, i64> = BTreeMap::new();
        let mut total: f64 = 0.0;
        for entry in &usage {
            let ms = entry.total_millis as f64;
            *by_date.entry(entry.date.clone()).or_insert(0.0) += ms;
            *by_app.entry(entry.app_class.to_string()).or_insert(0) += entry.total_millis;
            total += ms;
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

        // ── App list ──────────────────────────────────────────────────────────
        let meta: HashMap<&str, &str> = data
            .app_categories
            .iter()
            .map(|ac| (ac.app_class.as_str(), ac.display_name.as_str()))
            .collect();

        let mut app_list: Vec<ReportAppEntry> = by_app
            .into_iter()
            .map(|(app_class, total_millis)| {
                let display_name = meta
                    .get(app_class.as_str())
                    .copied()
                    .unwrap_or(&app_class)
                    .to_string();
                ReportAppEntry {
                    rank: 0,
                    total_millis,
                    percentage: if total > 0.0 {
                        (total_millis as f64 / total) * 100.0
                    } else {
                        0.0
                    },
                    app_class,
                    display_name,
                }
            })
            .collect();

        app_list.sort_by_key(|a| std::cmp::Reverse(a.total_millis));
        for (i, entry) in app_list.iter_mut().enumerate() {
            entry.rank = i + 1;
        }

        // ── Title list ────────────────────────────────────────────────────────
        let mut by_title: BTreeMap<(String, String), i64> = BTreeMap::new();
        let mut title_total: f64 = 0.0;
        for entry in &data.title_entries {
            let key = (entry.app_class.to_string(), entry.title.clone());
            *by_title.entry(key).or_insert(0) += entry.total_millis;
            title_total += entry.total_millis as f64;
        }

        let mut title_list: Vec<ReportTitleEntry> = by_title
            .into_iter()
            .map(|((app_class, title), total_millis)| {
                let display_name = meta
                    .get(app_class.as_str())
                    .copied()
                    .unwrap_or(&app_class)
                    .to_string();
                ReportTitleEntry {
                    rank: 0,
                    app_class: display_name,
                    title,
                    total_millis,
                    percentage: if title_total > 0.0 {
                        (total_millis as f64 / title_total) * 100.0
                    } else {
                        0.0
                    },
                }
            })
            .collect();

        title_list.sort_by_key(|a| std::cmp::Reverse(a.total_millis));
        for (i, entry) in title_list.iter_mut().enumerate() {
            entry.rank = i + 1;
        }

        // ── Top-level KPIs ────────────────────────────────────────────────────
        let total_millis = total as i64;
        let top_app = app_list
            .first()
            .map(|s| s.display_name.clone())
            .unwrap_or_else(|| "\u{2014}".into());

        self.bar_chart = bar_chart;
        self.app_list = app_list;
        self.title_list = title_list;
        self.total_millis = total_millis;
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
