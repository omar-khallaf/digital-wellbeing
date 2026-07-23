//! Dashboard ViewModel builder and data transformations.
//!
//! All functions are pure — no gpui, no async. Consumed by UI layer.

use std::collections::{BTreeMap, HashMap};

use chrono::NaiveDate;
use wellbeing_core::{AppCategoryRow, Category, DailySummary, DailyUsageEntry, DateRange};

use super::domain::{AppListEntry, BlockCardInfo, DashboardViewModel, Kpis};
use crate::chart::{Bar, Slice};

/// Transform D-Bus cache data into a `DashboardViewModel`.
pub fn build_dashboard_viewmodel(
    range: DateRange,
    summaries: &[DailySummary],
    categories: &[Category],
    app_categories: &[AppCategoryRow],
    block_cards: Vec<BlockCardInfo>,
) -> DashboardViewModel {
    let usage: Vec<DailyUsageEntry> = summaries
        .iter()
        .flat_map(|s| s.entries.iter().cloned())
        .collect();

    let bar_chart = build_bars(&usage);
    let pie_app = build_app_slices(&usage, app_categories);
    let pie_category = build_category_slices(&usage, categories, app_categories);
    let top_apps = build_top_apps(&usage, app_categories);

    DashboardViewModel {
        date_range: range,
        bar_chart,
        pie_app,
        pie_category,
        top_apps,
        block_cards,
    }
}

fn build_bars(usage: &[DailyUsageEntry]) -> Vec<Bar> {
    let mut by_date: BTreeMap<String, f64> = BTreeMap::new();
    for entry in usage {
        let total_millis = entry.total_millis as f64;
        *by_date.entry(entry.date.clone()).or_insert(0.0) += total_millis;
    }

    by_date
        .into_iter()
        .filter_map(|(date_str, total_millis)| {
            NaiveDate::parse_from_str(&date_str, "%Y-%m-%d")
                .ok()
                .map(|date| Bar { date, total_millis })
        })
        .collect()
}

fn build_app_slices(usage: &[DailyUsageEntry], app_categories: &[AppCategoryRow]) -> Vec<Slice> {
    let mut by_app: HashMap<String, f64> = HashMap::new();
    let mut total: f64 = 0.0;
    for entry in usage {
        let minutes = entry.total_millis as f64;
        *by_app.entry(entry.app_id.clone()).or_insert(0.0) += minutes;
        total += minutes;
    }

    if total <= 0.0 {
        return Vec::new();
    }

    let meta: HashMap<&str, &str> = app_categories
        .iter()
        .map(|ac| (ac.app_id.as_str(), ac.display_name.as_str()))
        .collect();

    let mut slices: Vec<Slice> = by_app
        .into_iter()
        .map(|(app_id, app_minutes)| {
            let display_name = meta
                .get(app_id.as_str())
                .copied()
                .unwrap_or(&app_id)
                .to_string();
            Slice {
                percentage: (app_minutes / total) * 100.0,
                app_id,
                display_name,
                color: String::new(),
            }
        })
        .collect();

    slices.sort_by(|a, b| {
        b.percentage
            .partial_cmp(&a.percentage)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    slices
}

fn build_category_slices(
    usage: &[DailyUsageEntry],
    categories: &[Category],
    app_categories: &[AppCategoryRow],
) -> Vec<Slice> {
    let app_to_cat: HashMap<&str, i64> = app_categories
        .iter()
        .map(|ac| (ac.app_id.as_str(), ac.category_id))
        .collect();

    let cat_map: HashMap<i64, &Category> = categories.iter().map(|c| (c.id.0, c)).collect();

    let mut by_cat: HashMap<String, (f64, String)> = HashMap::new();
    let mut total: f64 = 0.0;
    for entry in usage {
        let minutes = entry.total_millis as f64;
        let cat_id = app_to_cat.get(entry.app_id.as_str()).copied().unwrap_or(0);
        let cat_name = cat_map
            .get(&cat_id)
            .map(|c| c.name.clone())
            .unwrap_or_else(|| "Uncategorized".into());
        let entry = by_cat.entry(cat_name).or_insert((0.0, String::new()));
        entry.0 += minutes;
        total += minutes;
    }

    if total <= 0.0 {
        return Vec::new();
    }

    let mut slices: Vec<Slice> = by_cat
        .into_iter()
        .map(|(name, (cat_minutes, color))| Slice {
            percentage: (cat_minutes / total) * 100.0,
            app_id: name.clone(),
            display_name: name,
            color,
        })
        .collect();

    slices.sort_by(|a, b| {
        b.percentage
            .partial_cmp(&a.percentage)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    slices
}

fn build_top_apps(
    usage: &[DailyUsageEntry],
    app_categories: &[AppCategoryRow],
) -> Vec<AppListEntry> {
    let app_meta: HashMap<&str, (&str, Option<String>)> = app_categories
        .iter()
        .map(|ac| {
            let color = if ac.category_id > 0 {
                Some(format!("cat_{}", ac.category_id))
            } else {
                None
            };
            (ac.app_id.as_str(), (ac.display_name.as_str(), color))
        })
        .collect();

    let mut by_app: BTreeMap<String, i64> = BTreeMap::new();
    let mut grand_total: f64 = 0.0;
    for entry in usage {
        *by_app.entry(entry.app_id.clone()).or_insert(0) += entry.total_millis;
        grand_total += entry.total_millis as f64;
    }

    if grand_total <= 0.0 {
        return Vec::new();
    }

    let mut entries: Vec<AppListEntry> = by_app
        .into_iter()
        .map(|(app_id, total_millis)| {
            let (display_name, category_color) = app_meta
                .get(app_id.as_str())
                .map(|(name, color)| (name.to_string(), color.clone()))
                .unwrap_or_else(|| (app_id.clone(), None));
            AppListEntry {
                rank: 0,
                total_millis,
                percentage: (total_millis as f64 / grand_total) * 100.0,
                app_id,
                display_name,
                category_color,
                is_blocked: false,
            }
        })
        .collect();

    entries.sort_by_key(|a| std::cmp::Reverse(a.total_millis));
    for (i, entry) in entries.iter_mut().enumerate() {
        entry.rank = i + 1;
    }

    entries.truncate(10);
    entries
}

/// Compute the KPI summary for the stat row.
pub fn compute_kpis(vm: &DashboardViewModel) -> Kpis {
    let total_millis: i64 = vm.bar_chart.iter().map(|b| b.total_millis as i64).sum();
    let top = vm.top_apps.first();
    Kpis {
        total_millis,
        top_app: top
            .map(|t| t.display_name.clone())
            .unwrap_or_else(|| "\u{2014}".into()),
        top_app_millis: top.map(|t| t.total_millis).unwrap_or(0),
        active_blocks: vm.block_cards.len(),
    }
}
