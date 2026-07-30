use crate::valuetypes::*;
use serde::{Deserialize, Serialize};
use zvariant::Type;

/// Aggregated per-app usage across a date range — one row per app.
/// Returned pre-sorted by `total_millis` DESC from the daemon.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct AppUsageSummary {
    pub app_class: AppClass,
    pub total_millis: i64,
}

/// Aggregated per-title usage across a date range — one row per (app, title).
/// Returned pre-sorted by `total_millis` DESC from the daemon.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct TitleUsageSummary {
    pub app_class: AppClass,
    pub title: String,
    pub total_millis: i64,
}

/// Aggregated per-category usage across a date range — one row per category.
/// Returned pre-sorted by `total_millis` DESC from the daemon.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct CategoryUsageSummary {
    pub category: Category,
    pub total_millis: i64,
}

/// Per-date total usage across a date range — one row per date.
/// Pre-sorted by date ASC from SQL. Used for the bar chart so the GUI
/// doesn't need to flatten per-entry data and aggregate by date in memory.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct DateTotal {
    pub date: String,
    pub total_millis: i64,
}
