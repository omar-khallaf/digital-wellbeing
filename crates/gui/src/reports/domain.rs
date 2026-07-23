//! Reports domain types — pure data structures, no gpui dependency.
//! `Bar` lives in `crate::chart` (shared with Dashboard).

use wellbeing_core::DateRange;

use crate::chart::Bar;

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
    pub bar_chart: Vec<Bar>,
    pub app_list: Vec<ReportAppEntry>,
    pub total_millis: i64,
    pub top_app: String,
}
