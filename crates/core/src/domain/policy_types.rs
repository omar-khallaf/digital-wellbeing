use crate::valuetypes::*;
use chrono::{DateTime, Datelike, Timelike, Utc};
use serde::{Deserialize, Serialize};
use serde_repr::{Deserialize_repr, Serialize_repr};
use zvariant::Type;

/// Policy action discriminant — maps to DB integer.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize_repr, Deserialize_repr, Type)]
pub enum PolicyKind {
    Block = 0,
    TimeLimit = 1,
    Notify = 2,
}

/// Single time-window rule for schedule-based policy enforcement.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeWindow {
    pub start_hour: u8,
    pub end_hour: u8,
    #[serde(default)]
    pub days: Vec<u8>,
}

impl TimeWindow {
    pub fn is_active(&self, now: DateTime<Utc>) -> bool {
        if !self.days.is_empty() {
            let day_num = now.weekday().num_days_from_sunday() as u8;
            if !self.days.contains(&day_num) {
                return false;
            }
        }
        let hour = now.hour() as u8;
        if self.start_hour <= self.end_hour {
            hour >= self.start_hour && hour < self.end_hour
        } else {
            hour >= self.start_hour || hour < self.end_hour
        }
    }
}

/// Full policy as exposed over D-Bus.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct PolicyData {
    pub id: PolicyId,
    pub name: String,
    pub action: PolicyKind,
    pub app_id: String,
    pub category_id: i64,
    pub time_limit_minutes: i64,
    pub notification_repeat_interval_minutes: i64,
    pub schedule_json: String,
    pub active: bool,
    pub created_by: u32,
    pub owner_id: u32,
    pub created_at: String,
    pub updated_at: String,
}

/// Input for creating/updating a policy.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct PolicyInput {
    pub name: String,
    pub action: PolicyKind,
    pub app_id: String,
    pub category_id: i64,
    pub time_limit_minutes: i64,
    pub notification_repeat_interval_minutes: i64,
    pub schedule_json: String,
    pub active: bool,
    pub owner_id: u32,
}
