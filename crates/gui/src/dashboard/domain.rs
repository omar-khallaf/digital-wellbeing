//! Dashboard domain types — pure data structures, no gpui dependency.
//! `Bar` and `Slice` live in `crate::chart` (shared with Reports).

use chrono::{DateTime, NaiveDate, Utc};
use wellbeing_core::DateRange;

use crate::chart::Slice;

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

/// A single block in the day-timeline — either a focus interval or a gap.
#[derive(Debug, Clone)]
pub struct TimelineBlock {
    pub app_id: String,
    pub display_name: String,
    /// Start of this block (UTC epoch millis as DateTime).
    pub start: DateTime<Utc>,
    /// End of this block. `None` if currently-open (last focus has no close).
    pub end: Option<DateTime<Utc>>,
    /// Raw event_type from the events table.
    pub event_type: u8,
    /// True if this is an idle/untracked gap (not a focus interval).
    pub is_gap: bool,
}

/// The full day timeline, with per-app focus blocks and gaps.
#[derive(Debug, Clone)]
pub struct DayTimeline {
    pub date: NaiveDate,
    pub blocks: Vec<TimelineBlock>,
    /// Sum of all focus-interval millis (excluding gaps).
    pub total_focus_millis: i64,
}

/// Pure-data ViewModel for the Dashboard screen.
///
/// No gpui types, `Send + 'static`.  Built by `build_dashboard_viewmodel()`
/// from raw D-Bus cache data and consumed by gpui component constructors.
#[derive(Debug, Clone)]
pub struct DashboardViewModel {
    /// Inclusive date range for the current view.
    pub date_range: DateRange,
    /// Per-app usage breakdown (pie chart data).
    pub pie_app: Vec<Slice>,
    /// Per-category usage breakdown (pie chart data).
    pub pie_category: Vec<Slice>,
    /// Top N apps sorted by total usage.
    pub top_apps: Vec<AppListEntry>,
    /// Currently-blocked apps (info-only cards).
    pub block_cards: Vec<BlockCardInfo>,
    /// Day-timeline focus blocks and gaps (optional, loaded on demand).
    pub day_timeline: Option<DayTimeline>,
}

/// Computed KPI summary for the stat row.
#[derive(Debug, Clone)]
pub struct Kpis {
    pub total_millis: i64,
    pub top_app: String,
    pub top_app_millis: i64,
    pub active_blocks: usize,
}

/// A single fragment within an hourly bucket — either a focus interval or idle gap.
#[derive(Debug, Clone)]
pub struct TimelineFragment {
    pub app_id: String,
    pub display_name: String,
    pub millis: i64,
    pub is_gap: bool,
    /// Millis from the hour boundary to this fragment's start
    /// (e.g. a fragment at 05:36 in hour 5 has start_offset = 36 min).
    pub start_offset: i64,
}

/// One hourly bucket in the day timeline.
#[derive(Debug, Clone)]
pub struct HourlyBucket {
    pub hour: u32,
    /// All fragments (focus + gap) sorted by start_offset ASC.
    pub fragments: Vec<TimelineFragment>,
    pub total_millis: i64,
}
