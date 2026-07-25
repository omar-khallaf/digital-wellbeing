//! Repository struct definition for blocking-feature persistence.

use crate::store::DbPool;

/// Repository for blocking-feature persistence operations.
pub(crate) struct BlockingRepo {
    pub(crate) pool: DbPool,
}

impl BlockingRepo {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }
}

#[derive(Debug, Clone, diesel::Queryable, diesel::Selectable)]
#[diesel(table_name = crate::store::schema::events)]
pub struct EventRow {
    pub id: i32,
    pub event_type: i32,
    pub user_id: i32,
    pub timestamp: i64,
    pub app_id: Option<String>,
    pub title: Option<String>,
}
