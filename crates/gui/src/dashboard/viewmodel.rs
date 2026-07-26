//! Dashboard ViewModel builder — transforms D-Bus cache data into UI state.
//!
//! Pure functions with no gpui or async dependencies.

use std::collections::HashMap;

use wellbeing_core::{
    AppCategoryRow, Category, DailyUsageByAppEntry, DailyUsageByCategoryEntry,
    DailyUsageByTitleEntry,
};

use super::data::DashboardData;
use super::domain::{
    AppListEntry, BlockCardInfo, DashboardViewModel, DayTimeline, Kpis, TitleListEntry,
};
use crate::chart::Slice;

/// Transform D-Bus cache data into a `DashboardViewModel`.
pub fn build_dashboard_viewmodel(
    range: wellbeing_core::DateRange,
    data: &DashboardData,
    block_cards: Vec<BlockCardInfo>,
    day_timeline: Option<DayTimeline>,
) -> DashboardViewModel {
    let usage: Vec<DailyUsageByAppEntry> = data
        .summaries
        .iter()
        .flat_map(|s| s.entries.iter().cloned())
        .collect();

    let pie_app = build_app_slices(&usage, &data.app_categories);
    let pie_category = build_category_slices(&data.category_entries, &data.categories);
    let top_apps = build_top_apps(&usage, &data.app_categories);
    let top_titles = build_top_titles(&data.title_entries);

    DashboardViewModel {
        date_range: range,
        pie_app,
        pie_category,
        top_apps,
        block_cards,
        day_timeline,
        top_titles,
    }
}

fn build_app_slices(
    usage: &[DailyUsageByAppEntry],
    app_categories: &[AppCategoryRow],
) -> Vec<Slice> {
    let mut by_app: HashMap<String, f64> = HashMap::new();
    let mut total: f64 = 0.0;
    for entry in usage {
        let minutes = entry.total_millis as f64;
        *by_app.entry(entry.app_class.to_string()).or_insert(0.0) += minutes;
        total += minutes;
    }

    if total <= 0.0 {
        return Vec::new();
    }

    let meta: HashMap<&str, &str> = app_categories
        .iter()
        .map(|ac| (ac.app_class.as_str(), ac.display_name.as_str()))
        .collect();

    let mut slices: Vec<Slice> = by_app
        .into_iter()
        .map(|(app_class, app_minutes)| {
            let display_name = meta
                .get(app_class.as_str())
                .copied()
                .unwrap_or(&app_class)
                .to_string();
            Slice {
                percentage: (app_minutes / total) * 100.0,
                app_class,
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
    category_entries: &[DailyUsageByCategoryEntry],
    categories: &[Category],
) -> Vec<Slice> {
    let cat_color: HashMap<&str, &str> = categories
        .iter()
        .map(|c| (c.name.as_str(), c.color.as_str()))
        .collect();

    let mut by_cat: HashMap<String, (f64, String)> = HashMap::new();
    let mut total: f64 = 0.0;
    for entry in category_entries {
        let millis = entry.total_millis as f64;
        let color = cat_color
            .get(entry.category_name.as_str())
            .map(|c| c.to_string())
            .unwrap_or_default();
        let slot = by_cat
            .entry(entry.category_name.clone())
            .or_insert((0.0, color));
        slot.0 += millis;
        total += millis;
    }

    if total <= 0.0 {
        return Vec::new();
    }

    let mut slices: Vec<Slice> = by_cat
        .into_iter()
        .map(|(name, (cat_millis, color))| Slice {
            percentage: (cat_millis / total) * 100.0,
            app_class: name.clone(),
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
    usage: &[DailyUsageByAppEntry],
    app_categories: &[AppCategoryRow],
) -> Vec<AppListEntry> {
    let app_meta: HashMap<&str, (&str, Option<String>)> = app_categories
        .iter()
        .map(|ac| {
            let color = if ac.category_id.0 > 0 {
                Some(format!("cat_{}", ac.category_id.0))
            } else {
                None
            };
            (ac.app_class.as_str(), (ac.display_name.as_str(), color))
        })
        .collect();

    let mut by_app: HashMap<String, i64> = HashMap::new();
    let mut grand_total: f64 = 0.0;
    for entry in usage {
        *by_app.entry(entry.app_class.to_string()).or_insert(0) += entry.total_millis;
        grand_total += entry.total_millis as f64;
    }

    if grand_total <= 0.0 {
        return Vec::new();
    }

    let mut entries: Vec<AppListEntry> = by_app
        .into_iter()
        .map(|(app_class, total_millis)| {
            let (display_name, category_color) = app_meta
                .get(app_class.as_str())
                .map(|(name, color)| (name.to_string(), color.clone()))
                .unwrap_or_else(|| (app_class.clone(), None));
            AppListEntry {
                rank: 0,
                total_millis,
                percentage: (total_millis as f64 / grand_total) * 100.0,
                app_class,
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

fn build_top_titles(entries: &[DailyUsageByTitleEntry]) -> Vec<TitleListEntry> {
    let mut by_title: HashMap<(String, String), i64> = HashMap::new();
    let mut grand_total: f64 = 0.0;
    for entry in entries {
        let key = (entry.app_class.to_string(), entry.title.clone());
        *by_title.entry(key).or_insert(0) += entry.total_millis;
        grand_total += entry.total_millis as f64;
    }

    if grand_total <= 0.0 {
        return Vec::new();
    }

    let mut result: Vec<TitleListEntry> = by_title
        .into_iter()
        .map(|((app_class, title), total_millis)| TitleListEntry {
            rank: 0,
            app_class,
            title,
            total_millis,
            percentage: (total_millis as f64 / grand_total) * 100.0,
        })
        .collect();

    result.sort_by_key(|e| std::cmp::Reverse(e.total_millis));
    for (i, entry) in result.iter_mut().enumerate() {
        entry.rank = i + 1;
    }

    result.truncate(10);
    result
}

/// Compute the KPI summary for the stat row.
pub fn compute_kpis(vm: &DashboardViewModel) -> Kpis {
    let total_millis: i64 = vm.top_apps.iter().map(|a| a.total_millis).sum();
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
