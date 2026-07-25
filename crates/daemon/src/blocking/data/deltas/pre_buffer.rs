//! Pre-buffer event resolution.
//!
//! Fetches the last persisted event per uid before the buffer window so the
//! in-memory pairing loop in `apply_closed_deltas_from_buffer` can determine
//! whether each uid has an open interval that needs closing.

use std::collections::HashMap;

use diesel::{ExpressionMethods, QueryDsl};
use diesel_async::AsyncConnection;
use diesel_async::RunQueryDsl;
use wellbeing_core::Uid;

use crate::store::schema;

use super::super::repo::EventRow;
use super::BlockingRepo;

impl BlockingRepo {
    /// Fetch the last persisted event (any type) for each uid.
    ///
    /// Uses one query per uid. In practice the buffer typically contains 1-3
    /// distinct uids, so this is a bounded 1-3 round trips rather than O(N).
    pub(super) async fn fetch_last_events_for_uids<Conn>(
        conn: &mut Conn,
        uids: &[Uid],
    ) -> HashMap<Uid, Option<EventRow>>
    where
        Conn: AsyncConnection<Backend = diesel::sqlite::Sqlite>,
    {
        let mut results = HashMap::with_capacity(uids.len());
        for uid in uids {
            let row = schema::events::table
                .filter(schema::events::user_id.eq(uid.0 as i32))
                .order(schema::events::timestamp.desc())
                .limit(1)
                .first::<EventRow>(conn)
                .await
                .ok();
            results.insert(*uid, row);
        }
        results
    }
}
