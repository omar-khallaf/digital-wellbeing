//! Dashboard ViewModel — transforms D-Bus data into UI state.
//!
//! Pure functions with no gpui or async dependencies.  `DashboardViewModel`
//! methods mutate the VM in-place, like a Compose ViewModel with StateFlow.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use wellbeing_core::{
    AppCategoryRow, BlockedAppEntry, Category, DailyUsageByAppEntry, DailyUsageByCategoryEntry,
    DailyUsageByTitleEntry, DateRange,
};

use super::domain::{AppListEntry, BlockCardInfo, DashboardViewModel, Kpis, TitleListEntry};
use crate::chart::Slice;
use crate::dashboard::timeline::build_day_timeline;

// ---------------------------------------------------------------------------
// ViewModel mutation methods (Compose-like StateFlow pattern)
// ---------------------------------------------------------------------------

impl DashboardViewModel {
    /// Recompute ALL derived fields from the raw data in `self.data`.
    ///
    /// Call after a full fetch (DailyUsageChanged / daemon reconnect / manual
    /// refresh).  Mutates `day_events` in-place (sorts into timeline order).
    pub fn recompute_derived(&mut self) {
        let Some(ref mut data) = self.data else {
            self.clear_derived();
            return;
        };

        let usage: Vec<DailyUsageByAppEntry> = data
            .summaries
            .iter()
            .flat_map(|s| s.entries.iter().cloned())
            .collect();

        let app_names: HashMap<String, String> = data
            .app_categories
            .iter()
            .map(|ac| (ac.app_class.to_string(), ac.display_name.clone()))
            .collect();

        self.pie_app = build_app_slices(&usage, &data.app_categories);
        self.pie_category = build_category_slices(&data.category_entries, &data.categories);
        self.top_apps = build_top_apps(&usage, &data.app_categories);
        self.top_titles = build_top_titles(&data.title_entries);
        self.block_cards = build_block_cards(&data.blocked, &app_names);

        let today = Utc::now().date_naive();
        self.day_timeline = Some(build_day_timeline(
            &mut data.day_events, // sorted in-place — fine, this is VM state
            today,
            &app_names,
        ));
        self.date_range = DateRange {
            start: today,
            end: today,
        };
    }

    /// Fast path — only recompute `block_cards` from `data.blocked`.
    ///
    /// Call after `BlockedAppsChanged` signal.  Leaves all other derived
    /// fields untouched (they're still fresh from the last full fetch).
    pub fn recompute_blocked(&mut self) {
        let Some(ref data) = self.data else { return };

        let app_names: HashMap<String, String> = data
            .app_categories
            .iter()
            .map(|ac| (ac.app_class.to_string(), ac.display_name.clone()))
            .collect();

        self.block_cards = build_block_cards(&data.blocked, &app_names);
    }

    fn clear_derived(&mut self) {
        self.pie_app.clear();
        self.pie_category.clear();
        self.top_apps.clear();
        self.top_titles.clear();
        self.block_cards.clear();
        self.day_timeline = None;
    }
}

// ---------------------------------------------------------------------------
// Block card builder (extracted for use by both recompute paths)
// ---------------------------------------------------------------------------

pub fn build_block_cards(
    blocked: &[BlockedAppEntry],
    app_names: &HashMap<String, String>,
) -> Vec<BlockCardInfo> {
    blocked
        .iter()
        .map(|b| BlockCardInfo {
            app_class: b.app_class.to_string(),
            display_name: app_names
                .get(b.app_class.as_str())
                .cloned()
                .unwrap_or_default(),
            blocked_since: DateTime::from_timestamp_millis(b.blocked_since as i64)
                .unwrap_or(Utc::now()),
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Pure computation helpers (unchanged)
// ---------------------------------------------------------------------------

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
