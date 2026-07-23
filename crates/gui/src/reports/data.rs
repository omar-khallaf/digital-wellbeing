//! Reports ViewModel builder — pure function, no gpui.

use std::collections::BTreeMap;

use wellbeing_core::{AppCategoryRow, Category, DailySummary, DailyUsageEntry, DateRange};

use crate::chart::Bar;

use super::domain::{ReportAppEntry, ReportsViewModel};

/// Build a [`ReportsViewModel`] from cached usage data over the given [`DateRange`].
pub fn build_reports_viewmodel(
    range: DateRange,
    summaries: &[DailySummary],
    _: &[Category],
    app_categories: &[AppCategoryRow],
) -> ReportsViewModel {
    let usage: Vec<DailyUsageEntry> = summaries
        .iter()
        .flat_map(|s| s.entries.iter().cloned())
        .collect();

    // Bars — per-day totals (format_duration converts at display).
    let mut by_date: BTreeMap<String, f64> = BTreeMap::new();
    for entry in &usage {
        let ms = entry.total_millis as f64;
        *by_date.entry(entry.date.clone()).or_insert(0.0) += ms;
    }
    let bar_chart: Vec<Bar> = by_date
        .into_iter()
        .filter_map(|(d, m)| {
            chrono::NaiveDate::parse_from_str(&d, "%Y-%m-%d")
                .ok()
                .map(|date| Bar {
                    date,
                    total_millis: m,
                })
        })
        .collect();

    let mut by_app: BTreeMap<String, i64> = BTreeMap::new();
    let mut total: f64 = 0.0;
    for entry in &usage {
        *by_app.entry(entry.app_id.clone()).or_insert(0) += entry.total_millis;
        total += entry.total_millis as f64;
    }

    let meta: std::collections::HashMap<&str, &str> = app_categories
        .iter()
        .map(|ac| (ac.app_id.as_str(), ac.display_name.as_str()))
        .collect();

    let mut app_list: Vec<ReportAppEntry> = by_app
        .into_iter()
        .map(|(app_id, total_millis)| {
            let display_name = meta
                .get(app_id.as_str())
                .copied()
                .unwrap_or(&app_id)
                .to_string();
            ReportAppEntry {
                rank: 0,
                total_millis,
                percentage: if total > 0.0 {
                    (total_millis as f64 / total) * 100.0
                } else {
                    0.0
                },
                app_id,
                display_name,
            }
        })
        .collect();

    app_list.sort_by_key(|a| std::cmp::Reverse(a.total_millis));
    for (i, entry) in app_list.iter_mut().enumerate() {
        entry.rank = i + 1;
    }
    // No truncation — show ALL apps.

    let total_millis = total as i64;
    let top_app = app_list
        .first()
        .map(|s| s.display_name.clone())
        .unwrap_or_else(|| "\u{2014}".into());

    ReportsViewModel {
        date_range: range,
        bar_chart,
        app_list,
        total_millis,
        top_app,
    }
}
