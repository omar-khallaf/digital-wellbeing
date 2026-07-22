//! Reports domain types — pure data structures, no gpui dependency.
//! `Bar` and `Slice` live in `crate::chart` (shared with Dashboard).

use wellbeing_core::DateRange;

use crate::chart::{Bar, Slice};

/// ViewModel for the reports screen.
#[derive(Debug, Clone)]
pub struct ReportsViewModel {
    pub date_range: DateRange,
    pub bar_chart: Vec<Bar>,
    pub pie_app: Vec<Slice>,
    pub total_minutes: i64,
    pub top_app: String,
}
