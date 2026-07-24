//! Reports domain types — pure data structures, no gpui dependency.

use chrono::NaiveDate;
use wellbeing_core::DateRange;

/// One bar in the reports daily bar chart — one per day, height = hours tracked.
#[derive(Debug, Clone)]
pub struct DailyBar {
    pub date: NaiveDate,
    /// Total focus hours for this day (hourly millis / 3_600_000).
    pub hours_tracked: f64,
    /// Whether this bar represents today.
    pub is_today: bool,
}

/// A single row in the reports all-apps list.
#[derive(Debug, Clone)]
pub struct ReportAppEntry {
    pub rank: usize,
    pub app_id: String,
    pub display_name: String,
    pub total_millis: i64,
    pub percentage: f64,
}

/// ViewModel for the reports screen.
#[derive(Debug, Clone)]
pub struct ReportsViewModel {
    pub date_range: DateRange,
    pub bar_chart: Vec<DailyBar>,
    pub app_list: Vec<ReportAppEntry>,
    pub total_millis: i64,
    pub top_app: String,
}
