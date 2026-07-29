//! Dashboard ViewModel — transforms D-Bus data into UI state.
//!
//! Pure functions with no gpui or async dependencies.  `DashboardViewModel`
//! methods mutate the VM in-place, like a Compose ViewModel with StateFlow.
//!
//! App/title lists now use the pre-aggregated `app_summary` / `title_summary`
//! from the daemon (GROUP BY + ORDER BY total_millis DESC in SQL via the
//! generated `total_millis` column) — no GUI-side sorting or aggregation.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use wellbeing_core::{
    AppCategoryRow, AppUsageSummary, BlockedAppEntry, Category, CategoryUsageSummary, DateRange,
    TitleUsageSummary,
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

        let app_names: HashMap<String, String> = data
            .app_categories
            .iter()
            .map(|ac| (ac.app_class.to_string(), ac.display_name.clone()))
            .collect();

        // Pie charts use pre-aggregated summaries from SQL
        self.pie_app = build_app_slices(&data.app_summary, &data.app_categories);
        self.pie_category = build_category_slices(&data.category_summary, &data.categories);
        // Top apps/titles use pre-aggregated, pre-sorted data from daemon
        self.top_apps =
            build_top_apps_from_summary(&data.app_summary, &data.app_categories, &data.blocked);
        self.top_titles = build_top_titles_from_summary(&data.title_summary);
        self.block_cards = build_block_cards(&data.blocked, &app_names);

        let now = Utc::now();
        let today = now.date_naive();
        self.day_timeline = Some(build_day_timeline(
            &mut data.day_events, // sorted in-place — fine, this is VM state
            today,
            &app_names,
            now,
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
    app_summary: &[AppUsageSummary],
    app_categories: &[AppCategoryRow],
) -> Vec<Slice> {
    if app_summary.is_empty() {
        return Vec::new();
    }

    let total: f64 = app_summary.iter().map(|s| s.total_millis as f64).sum();
    if total <= 0.0 {
        return Vec::new();
    }

    let meta: HashMap<&str, &str> = app_categories
        .iter()
        .map(|ac| (ac.app_class.as_str(), ac.display_name.as_str()))
        .collect();

    let mut slices: Vec<Slice> = app_summary
        .iter()
        .map(|s| {
            let app_class = s.app_class.as_ref().to_string();
            let display_name = meta
                .get(app_class.as_str())
                .copied()
                .unwrap_or(&app_class)
                .to_string();
            Slice {
                percentage: (s.total_millis as f64 / total) * 100.0,
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
    category_summary: &[CategoryUsageSummary],
    categories: &[Category],
) -> Vec<Slice> {
    if category_summary.is_empty() {
        return Vec::new();
    }

    let total: f64 = category_summary.iter().map(|s| s.total_millis as f64).sum();
    if total <= 0.0 {
        return Vec::new();
    }

    let cat_color: HashMap<&str, &str> = categories.iter().map(|c| (c.name(), c.color())).collect();

    let mut slices: Vec<Slice> = category_summary
        .iter()
        .map(|s| {
            let cat_name = s.category.name();
            let color = cat_color
                .get(cat_name)
                .map(|c| c.to_string())
                .unwrap_or_default();
            Slice {
                percentage: (s.total_millis as f64 / total) * 100.0,
                app_class: cat_name.to_owned(),
                display_name: cat_name.to_owned(),
                color,
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

/// Build top-10 apps from pre-aggregated, pre-sorted daemon data.
///
/// `app_summary` is already grouped and sorted by total_millis DESC in SQL
/// (using the generated `total_millis` column).  No GUI-side sorting or
/// HashMap aggregation needed — just display-name lookup, percentage
/// computation, and truncation to 10.
fn build_top_apps_from_summary(
    app_summary: &[AppUsageSummary],
    app_categories: &[AppCategoryRow],
    blocked: &[BlockedAppEntry],
) -> Vec<AppListEntry> {
    if app_summary.is_empty() {
        return Vec::new();
    }

    let app_meta: HashMap<&str, (&str, Option<String>)> = app_categories
        .iter()
        .map(|ac| {
            let color = if (ac.category as u8) > 0 {
                Some(format!("cat_{}", ac.category as u8))
            } else {
                None
            };
            (ac.app_class.as_str(), (ac.display_name.as_str(), color))
        })
        .collect();

    let blocked_set: std::collections::HashSet<&str> =
        blocked.iter().map(|b| b.app_class.as_ref()).collect();

    let grand_total: f64 = app_summary.iter().map(|a| a.total_millis).sum::<i64>() as f64;
    if grand_total <= 0.0 {
        return Vec::new();
    }

    // Data is already sorted by SQL (total_millis DESC), rank on iteration.
    app_summary
        .iter()
        .take(10) // LIMIT 10
        .enumerate()
        .map(|(i, s)| {
            let app_class = s.app_class.as_ref().to_string();
            let (display_name, category_color) = app_meta
                .get(app_class.as_str())
                .map(|(name, color)| (name.to_string(), color.clone()))
                .unwrap_or_else(|| (app_class.clone(), None));
            AppListEntry {
                rank: i + 1,
                total_millis: s.total_millis,
                percentage: (s.total_millis as f64 / grand_total) * 100.0,
                app_class: app_class.clone(),
                display_name,
                category_color,
                is_blocked: blocked_set.contains(app_class.as_str()),
            }
        })
        .collect()
}

/// Build top-10 titles from pre-aggregated, pre-sorted daemon data.
///
/// `title_summary` is already grouped and sorted by total_millis DESC in SQL.
fn build_top_titles_from_summary(title_summary: &[TitleUsageSummary]) -> Vec<TitleListEntry> {
    if title_summary.is_empty() {
        return Vec::new();
    }

    let grand_total: f64 = title_summary.iter().map(|t| t.total_millis).sum::<i64>() as f64;
    if grand_total <= 0.0 {
        return Vec::new();
    }

    // Data is already sorted by SQL, rank on iteration.
    title_summary
        .iter()
        .take(10) // LIMIT 10
        .enumerate()
        .map(|(i, s)| TitleListEntry {
            rank: i + 1,
            app_class: s.app_class.as_ref().to_string(),
            title: s.title.clone(),
            total_millis: s.total_millis,
            percentage: (s.total_millis as f64 / grand_total) * 100.0,
        })
        .collect()
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
