//! Pre-buffer event resolution.
//!
//! Fetches the last persisted event per uid before the buffer window so the
//! in-memory pairing loop in `apply_closed_deltas_from_buffer` can determine
//! whether each uid has an open interval that needs closing.
//!
//! # Why ignored types are excluded
//!
//! `fetch_last_events_for_uids` filters out
//! [`INTERVAL_MEASUREMENT_IGNORED_EVENT_TYPES`] (Idle 2, Resumed 3) because
//! those events do not open or close focus intervals.  Without this filter
//! an Idle or Resumed event can become the last persisted event between
//! buffer flushes, causing the *next* flush to seed `open_focus = None`.
//! The interval from the last Focus to the next buffer event is then
//! silently dropped — the prior flush stored it as `open_millis`, but the
//! subsequent flush overwrites `open_millis` without ever converting that
//! time to `closed_millis`.

use std::collections::HashMap;

use diesel::{BoolExpressionMethods, ExpressionMethods, QueryDsl};
use diesel_async::AsyncConnection;
use diesel_async::RunQueryDsl;
use wellbeing_core::Uid;
use wellbeing_core::event_types::INTERVAL_MEASUREMENT_IGNORED_EVENT_TYPES;

use crate::store::schema;

use super::super::repo::EventRow;
use super::BlockingRepo;

impl BlockingRepo {
    /// Fetch the last persisted **non-ignored** event for each uid.
    ///
    /// Ignored event types ([`INTERVAL_MEASUREMENT_IGNORED_EVENT_TYPES`]) are
    /// excluded so the caller can reliably determine whether the uid has an
    /// open focus interval that needs closing: if the last non-ignored event
    /// is `WindowFocused(0)` the interval is still open; if it is a close
    /// event the interval was terminated and no open interval exists.
    ///
    /// Uses one query per uid. In practice the buffer typically contains 1-3
    /// distinct uids, so this is a bounded 1-3 round trips rather than O(N).
    pub(crate) async fn fetch_last_events_for_uids<Conn>(
        conn: &mut Conn,
        uids: &[Uid],
    ) -> HashMap<Uid, Option<EventRow>>
    where
        Conn: AsyncConnection<Backend = diesel::sqlite::Sqlite>,
    {
        let mut results: HashMap<Uid, Option<EventRow>> =
            uids.iter().map(|&uid| (uid, None)).collect();

        if uids.is_empty() {
            return results;
        }

        let raw_uids: Vec<i32> = uids.iter().map(|uid| uid.0 as i32).collect();

        // Alias schema for the subquery
        let (e1, e2) = (schema::events::table, diesel::alias!(schema::events as e2));

        // Fetch ONLY the latest single row per UID
        let fetched_rows: Vec<EventRow> = e1
            .filter(schema::events::user_id.eq_any(&raw_uids))
            .filter(schema::events::event_type.ne_all(INTERVAL_MEASUREMENT_IGNORED_EVENT_TYPES))
            .filter(diesel::dsl::not(diesel::dsl::exists(
                e2.filter(
                    e2.field(schema::events::user_id)
                        .eq(schema::events::user_id),
                )
                .filter(
                    e2.field(schema::events::event_type)
                        .ne_all(INTERVAL_MEASUREMENT_IGNORED_EVENT_TYPES),
                )
                .filter(
                    e2.field(schema::events::timestamp)
                        .gt(schema::events::timestamp)
                        .or(e2
                            .field(schema::events::timestamp)
                            .eq(schema::events::timestamp)
                            .and(e2.field(schema::events::id).gt(schema::events::id))),
                ),
            )))
            .load::<EventRow>(conn)
            .await
            .unwrap_or_default();

        for row in fetched_rows {
            let uid = Uid(row.user_id as u32);
            results.insert(uid, Some(row));
        }

        results
    }
}
