//! Policy row structs — plain Rust types, no diesel derives.

/// Row type for the `policies` table (11 columns).
///
/// `category` stores the integer discriminant of the `Category` enum
/// (0=Productivity…6=Uncategorized) or `None` when target_type is not Category.
#[derive(Debug, Clone)]
pub(crate) struct PolicyTableRow {
    pub(crate) id: i32,
    pub(crate) name: String,
    pub(crate) priority: i32,
    pub(crate) effect: i32,
    pub(crate) target_type: i32,
    pub(crate) app_id: Option<i32>,
    pub(crate) category: Option<i32>,
    pub(crate) domain_pattern: Option<String>,
    pub(crate) time_limit_minutes: Option<i32>,
    pub(crate) user_id: i32,
    pub(crate) created_by: i32,
}

/// Row type for the `policy_schedules` table.
#[derive(Debug, Clone)]
pub(crate) struct ScheduleRow {
    pub(crate) policy_id: i32,
    pub(crate) start_minute: i32,
    pub(crate) end_minute: i32,
    pub(crate) day_mask: i32,
}
