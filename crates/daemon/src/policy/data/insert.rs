//! Diesel Insertable structs for policy CRUD.

use crate::store::schema::policies;

/// Insert a new policy. No id or timestamps — those are auto-generated.
#[derive(Debug, Clone, diesel::Insertable)]
#[diesel(table_name = policies)]
pub(crate) struct NewPolicy {
    pub(crate) name: String,
    pub(crate) action: i32,
    pub(crate) category_id: Option<i32>,
    pub(crate) app_id: Option<String>,
    pub(crate) created_by: i32,
    pub(crate) owner_id: i32,
    pub(crate) time_limit_minutes: Option<i32>,
    pub(crate) extra_minutes: i32,
    pub(crate) notification_repeat_interval_minutes: Option<i32>,
    pub(crate) schedule_start_hour: Option<i32>,
    pub(crate) schedule_end_hour: Option<i32>,
    pub(crate) schedule_days: String,
    pub(crate) active: bool,
    pub(crate) created_at: String,
    pub(crate) updated_at: String,
}

/// Update an existing policy. Requires id; all other fields are Option<>.
#[derive(Debug, Clone, diesel::AsChangeset)]
#[diesel(table_name = policies)]
pub(crate) struct UpdatePolicy {
    pub(crate) name: Option<String>,
    pub(crate) action: Option<i32>,
    pub(crate) category_id: Option<Option<i32>>,
    pub(crate) app_id: Option<Option<String>>,
    pub(crate) time_limit_minutes: Option<Option<i32>>,
    pub(crate) extra_minutes: Option<i32>,
    pub(crate) notification_repeat_interval_minutes: Option<Option<i32>>,
    pub(crate) schedule_start_hour: Option<Option<i32>>,
    pub(crate) schedule_end_hour: Option<Option<i32>>,
    pub(crate) schedule_days: Option<String>,
    pub(crate) active: Option<bool>,
    pub(crate) updated_at: String,
}
