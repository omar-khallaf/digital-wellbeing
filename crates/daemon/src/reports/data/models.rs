//! Diesel Queryable structs for reports.

use crate::store::schema::{daily_usage, events};

/// Row type for the `events` table.
#[derive(Debug, Clone, diesel::Queryable, diesel::Selectable)]
#[diesel(table_name = events)]
pub struct EventRow {
    pub id: i32,
    pub event_type: i32,
    pub payload: String,
    pub user_id: i32,
    pub timestamp: String,
    pub app_id: Option<String>,
}

/// Row type for the `daily_usage` table — replaces raw tuple usage.
#[derive(Debug, Clone, diesel::Queryable, diesel::Selectable)]
#[diesel(table_name = daily_usage)]
pub struct DailyUsageRow {
    pub date: String,
    pub user_id: i32,
    pub app_id: String,
    pub total_minutes: i32,
    pub extended: bool,
    #[allow(dead_code)]
    pub updated_at: String,
}
