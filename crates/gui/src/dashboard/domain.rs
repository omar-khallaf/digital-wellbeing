//! Dashboard domain types — pure data structures, no gpui dependency.
//! `Bar` and `Slice` live in `crate::chart` (shared with Reports).

use chrono::{DateTime, Utc};
use wellbeing_core::DateRange;

use crate::chart::{Bar, Slice};

/// A single row in the top-apps list.
#[derive(Debug, Clone)]
pub struct AppListEntry {
    pub rank: usize,
    pub app_id: String,
    pub display_name: String,
    pub total_millis: i64,
    pub percentage: f64,
    pub category_color: Option<String>,
    pub is_blocked: bool,
}

/// Information about a currently-blocked application.
#[derive(Debug, Clone)]
pub struct BlockCardInfo {
    pub app_id: String,
    pub display_name: String,
    pub blocked_since: DateTime<Utc>,
}

/// Pure-data ViewModel for the Dashboard screen.
///
/// No gpui types, `Send + 'static`.  Built by `build_dashboard_viewmodel()`
/// from raw D-Bus cache data and consumed by gpui component constructors.
#[derive(Debug, Clone)]
pub struct DashboardViewModel {
    /// Inclusive date range for the current view.
    pub date_range: DateRange,
    /// Per-day total screen time (bar chart data).
    pub bar_chart: Vec<Bar>,
    /// Per-app usage breakdown (pie chart data).
    pub pie_app: Vec<Slice>,
    /// Per-category usage breakdown (pie chart data).
    pub pie_category: Vec<Slice>,
    /// Top N apps sorted by total usage.
    pub top_apps: Vec<AppListEntry>,
    /// Currently-blocked apps (info-only cards).
    pub block_cards: Vec<BlockCardInfo>,
}

/// Computed KPI summary for the stat row.
#[derive(Debug, Clone)]
pub struct Kpis {
    pub total_millis: i64,
    pub top_app: String,
    pub top_app_millis: i64,
    pub active_blocks: usize,
}
