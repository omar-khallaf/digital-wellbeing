//! Shared domain types used across daemon ↔ GUI D-Bus interface.
//! Flat structs with sentinel values for zvariant 5 compat (no Option<T>).
//! Convert to proper domain types with Options at handler boundaries.

use crate::valuetypes::*;
use serde::{Deserialize, Serialize};
use zvariant::Type;

/// Policy kind discriminant — maps to DB integer.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
pub enum PolicyKind {
    Block = 0,
    TimeLimit = 1,
    Notify = 2,
}

/// Full policy as exposed over D-Bus.
/// 0 / empty string = None for optional fields.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct Policy {
    pub id: PolicyId,
    pub name: String,
    pub kind: PolicyKind,
    /// Empty string = no app target.
    pub app_id: String,
    /// 0 = no category target.
    pub category_id: i64,
    /// 0 = no time limit (Block kind).
    pub time_limit_seconds: i64,
    pub extra_seconds: i64,
    /// 0 = no repeat notification.
    pub notification_repeat_interval_seconds: i64,
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
    pub kind: PolicyKind,
    pub app_id: String,
    pub category_id: i64,
    pub time_limit_seconds: i64,
    pub extra_seconds: i64,
    pub notification_repeat_interval_seconds: i64,
    pub schedule_json: String,
    pub active: bool,
    pub owner_id: u32,
}

/// One row of daily usage per app.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct DailyUsageEntry {
    pub date: String,
    pub user_id: u32,
    pub app_id: String,
    pub total_seconds: i64,
    pub extended: bool,
}

/// Summary for a date range.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct DailySummary {
    pub date: String,
    pub user_id: u32,
    pub entries: Vec<DailyUsageEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct Category {
    pub id: CategoryId,
    pub name: String,
    pub color: String,
    pub icon: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct AppCategoryRow {
    pub app_id: String,
    pub user_id: u32,
    pub category_id: i64,
    pub display_name: String,
    pub icon_path: String,
    pub ignore: bool,
}

/// Current active window info.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct ActiveWindowInfo {
    pub app_id: String,
    pub title: String,
    pub pid: u32,
}

/// Why an app was blocked. D-Bus serialized as uint32_t.
/// Maps to C++ `wellbeing::BlockReason` in the plugin.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
pub enum BlockReason {
    AppTimeLimit = 0,
    CategoryTimeLimit = 1,
    AppBlock = 2,
    CategoryBlock = 3,
}

/// Block state for a currently blocked app.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct BlockStateInfo {
    pub uid: u32,
    pub app_id: String,
    pub blocked: bool,
    pub reason: BlockReason,
}

/// Window info emitted by plugin.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct WindowInfo {
    pub app_id: String,
    pub title: String,
    pub pid: u32,
    pub overlay_shown: bool,
}

/// Plugin session state.
/// variant: 0=NoSession, 1=Desktop, 2=App
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct SessionState {
    pub variant: u32,
    pub app_id: String,
    pub title: String,
    pub pid: u32,
    pub overlay_shown: bool,
}
