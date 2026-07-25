//! Event persistence — write/flush operations.

use diesel::ExpressionMethods;
use diesel_async::{AsyncConnection, RunQueryDsl};

use crate::blocking::domain::TimedEvent;
use crate::store::schema;

use super::repo::BlockingRepo;

impl BlockingRepo {
    /// Batch INSERT buffered events into the events table.
    ///
    /// Diesel's `Vec`-based multi-row insert is not supported for the SQLite
    /// backend, so we insert rows one at a time. The caller is responsible for
    /// wrapping this call (and the delta computation) in a single transaction,
    /// so individual inserts share the same transaction.
    pub async fn flush_events<Conn>(
        &self,
        conn: &mut Conn,
        events: &[TimedEvent],
    ) -> anyhow::Result<()>
    where
        Conn: AsyncConnection<Backend = diesel::sqlite::Sqlite>,
    {
        if events.is_empty() {
            return Ok(());
        }

        for timed in events {
            let event = &timed.event;
            let app_id = event.app_id().map(|a| a.as_ref());
            let title = event.title().map(|t| t.as_str());
            let uid = event.uid().0 as i32;

            diesel::insert_into(schema::events::table)
                .values((
                    schema::events::event_type.eq(event.event_type()),
                    schema::events::user_id.eq(uid),
                    schema::events::timestamp.eq(timed.timestamp.timestamp_millis()),
                    schema::events::app_id.eq(app_id),
                    schema::events::title.eq(title),
                ))
                .execute(conn)
                .await?;
        }

        Ok(())
    }
}
