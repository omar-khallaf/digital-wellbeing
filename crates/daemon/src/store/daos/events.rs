//! Event repository — the `events` table.
//!
//! The events table is append-only. Flushes insert rows one at a time
//! because diesel's Vec-based multi-row insert is not supported for SQLite.

use std::collections::HashMap;

use diesel::BoolExpressionMethods;
use diesel::ExpressionMethods;
use diesel::QueryDsl;
use diesel::SelectableHelper;
use diesel_async::RunQueryDsl;
use wellbeing_core::{AppClass, DayEventRow, EventType, Uid, WindowTitle};

use crate::blocking::domain::TimedEvent;
use crate::store::connection::DbConn;
use crate::store::schema;

/// Event row type.
#[derive(Debug, Clone, diesel::Queryable, diesel::Selectable)]
#[diesel(table_name = crate::store::schema::events)]
pub struct EventRow {
    // All fields are populated via diesel Queryable derivation.
    #[allow(dead_code)]
    pub id: i32,
    pub event_type: i32,
    pub user_id: i32,
    pub timestamp: i64,
    pub app_class: Option<String>,
    pub title: Option<String>,
}

/// Diesel-backed event repository — provides static `_conn` methods only.
pub(crate) struct EventDao;

// ── Internal / transaction-friendly methods ─────────────────────────────

impl EventDao {
    /// Flush events using an existing connection.
    pub(crate) async fn flush_events(
        conn: &mut DbConn,
        events: &[TimedEvent],
    ) -> anyhow::Result<()> {
        if events.is_empty() {
            return Ok(());
        }

        for timed in events {
            let event = &timed.event;
            let app_class = event.app_class().map(|a| a.as_ref());
            let title = event.title().map(|t| t.as_str());
            let uid = event.uid().0 as i32;

            diesel::insert_into(schema::events::table)
                .values((
                    schema::events::event_type.eq(i32::from(event.event_type())),
                    schema::events::user_id.eq(uid),
                    schema::events::timestamp.eq(timed.timestamp.timestamp_millis()),
                    schema::events::app_class.eq(app_class),
                    schema::events::title.eq(title),
                ))
                .execute(conn)
                .await?;
        }

        Ok(())
    }

    /// Get events for a user in a time range.
    pub(crate) async fn get_day_events(
        conn: &mut DbConn,
        uid: i32,
        start_millis: i64,
        end_millis: i64,
    ) -> anyhow::Result<Vec<DayEventRow>> {
        let rows: Vec<EventRow> = schema::events::table
            .filter(schema::events::user_id.eq(uid))
            .filter(schema::events::timestamp.ge(start_millis))
            .filter(schema::events::timestamp.lt(end_millis))
            .order(schema::events::timestamp.asc())
            .select(EventRow::as_select())
            .load(conn)
            .await?;

        Ok(rows
            .into_iter()
            .map(|row| DayEventRow {
                id: row.id as u64,
                event_type: EventType::try_from(row.event_type).unwrap_or(EventType::Focus),
                timestamp: row.timestamp,
                app_class: row
                    .app_class
                    .filter(|s| !s.is_empty())
                    .and_then(|s| AppClass::new(&s).ok())
                    .unwrap_or_else(|| AppClass::new("unknown").unwrap()),
                title: WindowTitle::new(&row.title.unwrap_or_default()),
                user_id: row.user_id as u64,
            })
            .collect())
    }

    /// Fetch last non-ignored events using an existing connection.
    pub(crate) async fn get_last_events_for_uids(
        conn: &mut DbConn,
        uids: &[Uid],
    ) -> HashMap<Uid, Option<EventRow>> {
        use diesel::dsl::{exists, not};
        use diesel::query_dsl::methods::FilterDsl;

        let mut results: HashMap<Uid, Option<EventRow>> =
            uids.iter().map(|&uid| (uid, None)).collect();

        if uids.is_empty() {
            return results;
        }

        let raw_uids: Vec<i32> = uids.iter().map(|uid| uid.0 as i32).collect();

        // Alias schema for the subquery
        let (e1, e2) = (schema::events::table, diesel::alias!(schema::events as e2));

        let ignored_types: Vec<i32> = EventType::IGNORED_BY_MEASUREMENT
            .iter()
            .map(|&et| et.into())
            .collect();

        // Fetch only the latest single row per UID, excluding ignored types.
        let fetched_rows: Vec<EventRow> = FilterDsl::filter(
            QueryDsl::filter(
                QueryDsl::filter(e1, schema::events::user_id.eq_any(&raw_uids)),
                schema::events::event_type.ne_all(&ignored_types),
            ),
            not(exists(FilterDsl::filter(
                FilterDsl::filter(
                    FilterDsl::filter(
                        e2,
                        e2.field(schema::events::user_id)
                            .eq(schema::events::user_id),
                    ),
                    e2.field(schema::events::event_type).ne_all(&ignored_types),
                ),
                e2.field(schema::events::timestamp)
                    .gt(schema::events::timestamp)
                    .or(e2
                        .field(schema::events::timestamp)
                        .eq(schema::events::timestamp)
                        .and(e2.field(schema::events::id).gt(schema::events::id))),
            ))),
        )
        .load(conn)
        .await
        .unwrap_or_default();

        for row in fetched_rows {
            let uid = Uid(row.user_id as u32);
            results.insert(uid, Some(row));
        }

        results
    }

    /// Get events within a timestamp range (inclusive).
    pub(crate) async fn get_event_range(
        conn: &mut DbConn,
        start: i64,
        end: i64,
    ) -> anyhow::Result<Vec<EventRow>> {
        use schema::events::columns as c;
        Ok(schema::events::table
            .filter(c::timestamp.ge(start))
            .filter(c::timestamp.le(end))
            .order(c::timestamp.asc())
            .select(EventRow::as_select())
            .load::<EventRow>(conn)
            .await?)
    }
}
