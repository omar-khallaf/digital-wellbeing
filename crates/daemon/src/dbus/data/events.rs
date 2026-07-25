//! Event query helpers for D-Bus interface.

use diesel::{ExpressionMethods, QueryDsl};
use diesel_async::RunQueryDsl;
use wellbeing_core::DayEventRow;

use crate::store::DbPool;
use crate::store::schema::events;

type EventRowRaw = (i32, i32, i32, i64, Option<String>, Option<String>);

pub(crate) async fn get_day_events(
    pool: &DbPool,
    uid: i32,
    start_millis: i64,
    end_millis: i64,
) -> anyhow::Result<Vec<DayEventRow>> {
    let mut conn = pool.get().await?;

    let rows: Vec<EventRowRaw> = events::table
        .filter(events::user_id.eq(uid))
        .filter(events::timestamp.ge(start_millis))
        .filter(events::timestamp.lt(end_millis))
        .order_by(events::timestamp.asc())
        .select((
            events::id,
            events::event_type,
            events::user_id,
            events::timestamp,
            events::app_id,
            events::title,
        ))
        .load(&mut conn)
        .await?;

    Ok(rows
        .into_iter()
        .map(|(id, event_type, user_id, ts, app_id, title)| DayEventRow {
            id: id as u64,
            event_type: event_type as u8,
            timestamp: ts,
            app_id: app_id.unwrap_or_default(),
            title: title.unwrap_or_default(),
            user_id: user_id as u64,
        })
        .collect())
}
