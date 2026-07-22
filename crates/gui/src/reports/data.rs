//! Reports ViewModel builder — pure function, no gpui.

use std::collections::{BTreeMap, HashMap};

use wellbeing_core::{AppCategoryRow, Category, DailySummary, DailyUsageEntry, DateRange};

use crate::chart::{Bar, Slice};

use super::domain::ReportsViewModel;

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

    // Bars — keep values in minutes (no /60).
    let mut by_date: BTreeMap<String, f64> = BTreeMap::new();
    for entry in &usage {
        let minutes = entry.total_minutes as f64;
        *by_date.entry(entry.date.clone()).or_insert(0.0) += minutes;
    }
    let bar_chart: Vec<Bar> = by_date
        .into_iter()
        .filter_map(|(d, m)| {
            chrono::NaiveDate::parse_from_str(&d, "%Y-%m-%d")
                .ok()
                .map(|date| Bar {
                    date,
                    total_minutes: m,
                })
        })
        .collect();

    // App slices — percentages only, divide cancels out.
    let mut by_app: HashMap<String, f64> = HashMap::new();
    let mut total: f64 = 0.0;
    for entry in &usage {
        let minutes = entry.total_minutes as f64;
        *by_app.entry(entry.app_id.clone()).or_insert(0.0) += minutes;
        total += minutes;
    }
    let pie_app: Vec<Slice> = if total <= 0.0 {
        Vec::new()
    } else {
        let meta: HashMap<&str, &str> = app_categories
            .iter()
            .map(|ac| (ac.app_id.as_str(), ac.display_name.as_str()))
            .collect();
        let mut slices: Vec<Slice> = by_app
            .into_iter()
            .map(|(app_id, minutes)| Slice {
                percentage: (minutes / total) * 100.0,
                app_id: app_id.clone(),
                display_name: meta
                    .get(app_id.as_str())
                    .copied()
                    .unwrap_or(&app_id)
                    .to_string(),
                color: String::new(),
            })
            .collect();
        slices.sort_by(|a, b| {
            b.percentage
                .partial_cmp(&a.percentage)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        slices
    };

    let total_minutes = total as i64;
    let top_app = pie_app
        .first()
        .map(|s| s.display_name.clone())
        .unwrap_or_else(|| "\u{2014}".into());

    ReportsViewModel {
        date_range: range,
        bar_chart,
        pie_app,
        total_minutes,
        top_app,
    }
}
