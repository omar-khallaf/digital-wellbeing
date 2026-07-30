//! Dashboard domain types — pure data structures, no gpui dependency.
//! `Bar` and `Slice` live in `crate::chart` (shared with Reports).

use chrono::{DateTime, NaiveDate, Utc};
use wellbeing_core::{DateRange, EventType};

use crate::chart::Slice;

use super::data::DashboardData;

#[derive(Debug, Clone, PartialEq)]
pub struct AppListEntry {
    pub rank: usize,
    pub app_class: String,
    pub display_name: String,
    pub total_millis: i64,
    pub percentage: f64,
    pub category_color: Option<String>,
    pub is_blocked: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TitleListEntry {
    pub rank: usize,
    pub app_class: String,
    pub title: String,
    pub total_millis: i64,
    pub percentage: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BlockCardInfo {
    pub app_class: String,
    pub display_name: String,
    pub blocked_since: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TimelineBlock {
    pub app_class: String,
    pub display_name: String,
    pub start: DateTime<Utc>,
    /// End of this block. `None` if currently-open (last focus has no close).
    pub end: Option<DateTime<Utc>>,
    /// Event type from the events table.
    pub event_type: EventType,
    pub is_gap: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DayTimeline {
    pub date: NaiveDate,
    pub blocks: Vec<TimelineBlock>,
    /// Sum of all focus-interval millis (excluding gaps).
    pub total_focus_millis: i64,
}

/// Pure-data ViewModel for the Dashboard screen.
///
/// No gpui types, `Send + 'static`.  Acts like a Compose ViewModel with
/// `StateFlow` — raw data persists in `self.data` and derived fields
/// are recomputed in-place via `recompute_derived()` / `recompute_blocked()`.
#[derive(Debug, Clone, PartialEq)]
pub struct DashboardViewModel {
    /// Raw data bundle (like `MutableStateFlow<DashboardData?>` in Kotlin).
    /// `None` before the first successful fetch.
    pub data: Option<DashboardData>,
    pub date_range: DateRange,
    pub pie_app: Vec<Slice>,
    pub pie_category: Vec<Slice>,
    pub top_apps: Vec<AppListEntry>,
    pub block_cards: Vec<BlockCardInfo>,
    /// Day-timeline focus blocks and gaps (optional, loaded on demand).
    pub day_timeline: Option<DayTimeline>,
    pub top_titles: Vec<TitleListEntry>,
}

impl Default for DashboardViewModel {
    fn default() -> Self {
        Self {
            data: None,
            date_range: DateRange {
                start: NaiveDate::default(),
                end: NaiveDate::default(),
            },
            pie_app: Vec::new(),
            pie_category: Vec::new(),
            top_apps: Vec::new(),
            block_cards: Vec::new(),
            day_timeline: None,
            top_titles: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Kpis {
    pub total_millis: i64,
    pub top_app: String,
    pub top_app_millis: i64,
    pub active_blocks: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TimelineFragment {
    pub app_class: String,
    pub display_name: String,
    pub millis: i64,
    pub is_gap: bool,
    /// Millis from the hour boundary to this fragment's start
    /// (e.g. a fragment at 05:36 in hour 5 has start_offset = 36 min).
    pub start_offset: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HourlyBucket {
    pub hour: u32,
    /// All fragments (focus + gap) sorted by start_offset ASC.
    pub fragments: Vec<TimelineFragment>,
    pub total_millis: i64,
}
