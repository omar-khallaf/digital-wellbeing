use crate::valuetypes::*;
use serde::{Deserialize, Serialize};
use zvariant::Type;

/// Per-app daily usage projection.
///
/// Fields use validated domain types where the D-Bus signature is preserved
/// (`AppClass` has the same `s` signature as `String`; `Uid` has the same `u`
/// signature as `u32`).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct DailyUsageByAppEntry {
    pub date: String,
    pub user_id: Uid,
    pub app_class: AppClass,
    pub total_millis: i64,
}

/// Per-title usage projection — one row per (date, user_id, app_class, title).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct DailyUsageByTitleEntry {
    pub date: String,
    pub user_id: Uid,
    pub app_class: AppClass,
    pub title: String,
    pub total_millis: i64,
}

/// Per-title usage summary for a date range — groups entries by date.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct DailyUsageByTitleSummary {
    pub date: String,
    pub user_id: Uid,
    pub entries: Vec<DailyUsageByTitleEntry>,
}

/// Per-category usage projection — one row per (date, user_id, category_name).
/// `category_name` is resolved at query time (the raw DB `category_id` is
/// internal plumbing and must never leak into the D-Bus interface).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct DailyUsageByCategoryEntry {
    pub date: String,
    pub user_id: Uid,
    pub category_name: String,
    pub total_millis: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct DailySummary {
    pub date: String,
    pub user_id: Uid,
    pub entries: Vec<DailyUsageByAppEntry>,
}
