//! App registry repository — the `apps` table.
//!
//! Uses a transactional get-or-insert pattern: SELECT first, then INSERT
//! with RETURNING if not found. No ON CONFLICT.

use diesel::ExpressionMethods;
use diesel::QueryDsl;
use diesel::dsl::insert_into;
use diesel_async::RunQueryDsl;
use wellbeing_core::AppClass;

use crate::store::connection::DbConn;
use crate::store::schema;
pub(crate) struct AppsDao;

// ── Internal / transaction-friendly methods ─────────────────────────────

impl AppsDao {
    /// Ensure an app exists using an existing connection (for use inside
    /// larger transactions).
    pub(crate) async fn ensure_app(conn: &mut DbConn, app_class: &AppClass) -> anyhow::Result<i32> {
        let existing: Option<i32> = schema::apps::table
            .filter(schema::apps::app_class.eq(app_class.as_ref()))
            .select(schema::apps::id)
            .first(conn)
            .await
            .ok();

        match existing {
            Some(id) => Ok(id),
            None => {
                let new_id: i32 = insert_into(schema::apps::table)
                    .values(schema::apps::app_class.eq(app_class.as_ref()))
                    .returning(schema::apps::id)
                    .get_result(conn)
                    .await?;
                Ok(new_id)
            }
        }
    }
}
