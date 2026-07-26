//! Diesel Queryable structs for the new policy engine schema.

/// Row type for the `policies` table (13 columns, no category name).
///
/// Used by regular CRUD queries that load directly from the policies table
/// without a categories join. Convert to `PolicyRow` via `.into()` when
/// the domain layer needs category-aware conversion.
#[derive(Debug, Clone, diesel::Queryable, diesel::Selectable)]
#[diesel(table_name = crate::store::schema::policies)]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub(crate) struct PolicyTableRow {
    pub(crate) id: i32,
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

/// Full policy row with resolved category name (14 fields).
///
/// Used by queries that perform a `LEFT JOIN categories c ON c.id = p.category_id`
/// to populate `category_name`. Loaded via `sql_query` + `QueryableByName`.
#[derive(Debug, Clone, diesel::QueryableByName)]
pub(crate) struct PolicyRow {
    #[diesel(sql_type = diesel::sql_types::Integer)]
    pub(crate) id: i32,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub(crate) name: String,
    #[diesel(sql_type = diesel::sql_types::Integer)]
    pub(crate) priority: i32,
    #[diesel(sql_type = diesel::sql_types::Integer)]
    pub(crate) effect: i32,
    #[diesel(sql_type = diesel::sql_types::Integer)]
    pub(crate) target_type: i32,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Integer>)]
    pub(crate) app_id: Option<i32>,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Integer>)]
    #[allow(dead_code)] // populated by diesel QueryableByName deserialization
    pub(crate) category_id: Option<i32>,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Text>)]
    pub(crate) domain_pattern: Option<String>,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Integer>)]
    pub(crate) time_limit_minutes: Option<i32>,
    #[diesel(sql_type = diesel::sql_types::Integer)]
    pub(crate) user_id: i32,
    #[diesel(sql_type = diesel::sql_types::Integer)]
    pub(crate) created_by: i32,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub(crate) created_at: String,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub(crate) updated_at: String,
    /// Category name resolved via `LEFT JOIN categories c ON c.id = p.category_id`.
    /// Only populated when `target_type = Category`; `None` otherwise.
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Text>)]
    pub(crate) category_name: Option<String>,
}

impl From<PolicyTableRow> for PolicyRow {
    fn from(row: PolicyTableRow) -> Self {
        Self {
            id: row.id,
            name: row.name,
            priority: row.priority,
            effect: row.effect,
            target_type: row.target_type,
            app_id: row.app_id,
            category_id: row.category_id,
            domain_pattern: row.domain_pattern,
            time_limit_minutes: row.time_limit_minutes,
            user_id: row.user_id,
            created_by: row.created_by,
            created_at: row.created_at,
            updated_at: row.updated_at,
            category_name: None,
        }
    }
}

/// Row type for the `policy_schedules` table.
#[derive(Debug, Clone, diesel::Queryable)]
pub(crate) struct ScheduleRow {
    pub(crate) policy_id: i32,
    pub(crate) start_minute: i32,
    pub(crate) end_minute: i32,
    pub(crate) day_mask: i32,
}
