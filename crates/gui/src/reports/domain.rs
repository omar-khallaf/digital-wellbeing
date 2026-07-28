//! Reports domain types — pure data structures, no gpui dependency.

use chrono::NaiveDate;
use wellbeing_core::DateRange;

use super::data::ReportsData;

/// One bar in the reports daily bar chart — one per day, height = hours tracked.
#[derive(Debug, Clone)]
pub struct DailyBar {
    pub date: NaiveDate,
    /// Total focus hours for this day (hourly millis / 3_600_000).
    pub hours_tracked: f64,
    pub is_today: bool,
}

/// A single row in the reports all-apps list.
#[derive(Debug, Clone)]
pub struct ReportAppEntry {
    pub rank: usize,
    pub app_class: String,
    pub display_name: String,
    pub total_millis: i64,
    pub percentage: f64,
}

/// A single row in the reports all-titles list.
#[derive(Debug, Clone)]
pub struct ReportTitleEntry {
    pub rank: usize,
    pub app_class: String,
    pub title: String,
    pub total_millis: i64,
    pub percentage: f64,
}

/// ViewModel for the reports screen.
///
/// Acts like a Compose ViewModel with `StateFlow` — raw data persists in
/// `self.data` and derived fields are recomputed via `recompute_derived()`.
#[derive(Debug, Clone)]
pub struct ReportsViewModel {
    /// Raw data bundle (like `MutableStateFlow<ReportsData?>`).
    /// `None` before the first successful fetch.
    pub data: Option<ReportsData>,
    pub date_range: DateRange,
    pub bar_chart: Vec<DailyBar>,
    pub app_list: Vec<ReportAppEntry>,
    pub title_list: Vec<ReportTitleEntry>,
    pub total_millis: i64,
    pub top_app: String,
}

impl Default for ReportsViewModel {
    fn default() -> Self {
        Self {
            data: None,
            date_range: DateRange {
                start: NaiveDate::default(),
                end: NaiveDate::default(),
            },
            bar_chart: Vec::new(),
            app_list: Vec::new(),
            title_list: Vec::new(),
            total_millis: 0,
            top_app: String::new(),
        }
    }
}
