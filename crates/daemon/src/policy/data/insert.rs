use crate::store::schema::policies;

/// Insertable row for the `policies` table.
#[derive(Debug, Clone, diesel::Insertable)]
#[diesel(table_name = policies)]
pub(crate) struct NewPolicy {
    pub(crate) name: String,
    pub(crate) priority: i32,
    pub(crate) effect: i32,
    pub(crate) target_type: i32,
    pub(crate) app_id: Option<i32>,
    pub(crate) category_id: Option<i32>,
    pub(crate) domain_pattern: Option<String>,
    pub(crate) time_limit_minutes: Option<i32>,
    pub(crate) user_id: i32,
    pub(crate) created_by: i32,
    pub(crate) created_at: String,
    pub(crate) updated_at: String,
}

/// Update struct for the `policies` table.
#[derive(Debug, Clone, diesel::AsChangeset)]
#[diesel(table_name = policies)]
pub(crate) struct UpdatePolicy {
    pub(crate) name: Option<String>,
    pub(crate) priority: Option<i32>,
    pub(crate) effect: Option<i32>,
    pub(crate) target_type: Option<i32>,
    pub(crate) app_id: Option<Option<i32>>,
    pub(crate) category_id: Option<Option<i32>>,
    pub(crate) domain_pattern: Option<Option<String>>,
    pub(crate) time_limit_minutes: Option<Option<i32>>,
    pub(crate) user_id: Option<i32>,
    pub(crate) updated_at: String,
}
