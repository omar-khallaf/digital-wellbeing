//! Diesel Queryable structs for reports.

use crate::store::schema::{daily_usage, daily_usage_by_title, events};

#[derive(Debug, Clone, diesel::Queryable, diesel::Selectable)]
#[diesel(table_name = events)]
pub struct EventRow {
    pub id: i32,
    pub event_type: i32,
    pub user_id: i32,
    pub timestamp: i64,
    pub app_id: Option<String>,
    pub title: Option<String>,
}

/// Row type for the `daily_usage` table — replaces raw tuple usage.
#[derive(Debug, Clone, diesel::Queryable, diesel::Selectable)]
#[diesel(table_name = daily_usage)]
pub struct DailyUsageRow {
    pub date: String,
    pub user_id: i32,
    pub app_id: String,
    pub closed_millis: i64,
    pub open_millis: i64,
}

#[derive(Debug, Clone, diesel::Queryable, diesel::Selectable)]
#[diesel(table_name = daily_usage_by_title)]
pub struct DailyUsageByTitleRow {
    pub date: String,
    pub user_id: i32,
    pub app_id: String,
    pub title: String,
    pub closed_millis: i64,
    pub open_millis: i64,
}

/// One hour bucket (0-23) with total focus milliseconds.
#[derive(Debug, Clone)]
pub struct HourlyUsageRow {
    pub hour: u8,
    pub total_millis: i64,
}
