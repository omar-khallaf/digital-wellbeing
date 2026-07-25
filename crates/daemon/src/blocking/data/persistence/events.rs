//! Event persistence — write/flush operations and BlockingRepo struct.

use diesel::ExpressionMethods;
use diesel_async::{AsyncConnection, RunQueryDsl};
use wellbeing_core::event_types::{
    EVENT_IDLE, EVENT_RESUMED, EVENT_WINDOW_BLOCKED, EVENT_WINDOW_FOCUSED,
};

use crate::store::{DbPool, schema};

use super::super::super::buffer::BufferedEvent;

/// Repository for blocking-feature persistence operations.
pub(crate) struct BlockingRepo {
    pub(crate) pool: DbPool,
}

impl BlockingRepo {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    /// Batch INSERT buffered events into the events table in a single transaction.
    /// Daily usage is NOT accumulated here; it is materialized by
    /// `apply_closed_deltas_from_buffer` in the same transaction.
    pub async fn flush_events<Conn>(
        &self,
        conn: &mut Conn,
        events: &[BufferedEvent],
    ) -> anyhow::Result<()>
    where
        Conn: AsyncConnection<Backend = diesel::sqlite::Sqlite>,
    {
        for event in events {
            let app_id = if event.event_type == EVENT_WINDOW_FOCUSED
                || event.event_type == EVENT_IDLE
                || event.event_type == EVENT_RESUMED
                || event.event_type == EVENT_WINDOW_BLOCKED
            {
                Some(event.app_id.as_ref())
            } else {
                None
            };
            let title = event.title.as_ref().map(|t| t.as_str());
            diesel::insert_into(schema::events::table)
                .values((
                    schema::events::event_type.eq(event.event_type),
                    schema::events::user_id.eq(event.uid.0 as i32),
                    schema::events::timestamp.eq(event.timestamp.timestamp_millis()),
                    schema::events::app_id.eq(app_id),
                    schema::events::title.eq(title),
                ))
                .execute(conn)
                .await?;
        }
        Ok(())
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
