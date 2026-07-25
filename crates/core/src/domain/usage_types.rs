use serde::{Deserialize, Serialize};
use zvariant::Type;

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct DailyUsageByAppEntry {
    pub date: String,
    pub user_id: u32,
    pub app_id: String,
    pub total_millis: i64,
}

/// Per-title usage projection — one row per (date, user_id, app_id, title).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct DailyUsageByTitleEntry {
    pub date: String,
    pub user_id: u32,
    pub app_id: String,
    pub title: String,
    pub total_millis: i64,
}

/// Per-title usage summary for a date range — groups entries by date.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct DailyUsageByTitleSummary {
    pub date: String,
    pub user_id: u32,
    pub entries: Vec<DailyUsageByTitleEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct DailySummary {
    pub date: String,
    pub user_id: u32,
    pub entries: Vec<DailyUsageByAppEntry>,
}
