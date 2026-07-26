//! Hourly usage aggregation and startup-interval recovery.

use chrono::{NaiveDate, TimeDelta};
use wellbeing_core::{EventType, Uid};

use crate::reports::data::models::HourlyUsageRow;
use crate::store::connection::DbConn;
use crate::store::daos::events::{EventDao, EventRow};

/// Returns exactly 24 rows (hours 0-23), missing hours filled with 0.
pub async fn get_hourly_usage(
    conn: &mut DbConn,
    user_id: Uid,
    date: NaiveDate,
) -> anyhow::Result<Vec<HourlyUsageRow>> {
    let start_millis = date
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_utc()
        .timestamp_millis();
    let end_millis = (date + TimeDelta::days(1))
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_utc()
        .timestamp_millis();

    let events = EventDao::get_event_range(conn, start_millis, end_millis).await?;

    let user_events: Vec<&EventRow> = events
        .iter()
        .filter(|e| e.user_id == user_id.0 as i32)
        .collect();

    let mut hourly = [0i64; 24];
    let mut focus_start: Option<i64> = None;

    for event in &user_events {
        if event.event_type == i32::from(EventType::Focus) {
            // Focus — record the start of a focus interval
            focus_start = Some(event.timestamp);
        } else if EventType::try_from(event.event_type).is_ok_and(|et| et.is_close()) {
            // Close event — process any pending interval
            if let Some(start_ts) = focus_start.take() {
                add_interval_to_hourly(&mut hourly, start_ts, event.timestamp, start_millis);
            }
        }
        // Idle(2) / Resumed(3) are ignored — neither start nor close intervals
    }

    // Unmatched focus at end of day: use end_millis as close time
    if let Some(start_ts) = focus_start {
        add_interval_to_hourly(&mut hourly, start_ts, end_millis, start_millis);
    }

    Ok(hourly
        .iter()
        .enumerate()
        .map(|(h, ms)| HourlyUsageRow {
            hour: h as u8,
            total_millis: *ms,
        })
        .collect())
}

/// Distribute a focus interval into per-hour buckets with cross-hour splitting.
fn add_interval_to_hourly(
    hourly: &mut [i64; 24],
    interval_start: i64,
    interval_end: i64,
    day_start_millis: i64,
) {
    let start_hour = ((interval_start - day_start_millis) / 3_600_000).max(0) as usize;
    let end_hour = ((interval_end - day_start_millis - 1) / 3_600_000).clamp(0, 23) as usize;

    for (offset, slot) in hourly[start_hour..=end_hour].iter_mut().enumerate() {
        let actual_h = start_hour + offset;
        let hour_start = day_start_millis + (actual_h as i64 * 3_600_000);
        let hour_end = hour_start + 3_600_000;
        let overlap_start = interval_start.max(hour_start);
        let overlap_end = interval_end.min(hour_end);
        let millis = (overlap_end - overlap_start).max(0);
        *slot += millis;
    }
}

/// Check if we need to open an interval at startup (crashed with active focus).
/// Returns `(app_class, timestamp_millis, is_paused)`.
pub async fn open_interval_at_startup(
    conn: &mut DbConn,
) -> anyhow::Result<Option<(String, i64, bool)>> {
    use crate::store::schema::events;
    use diesel::{ExpressionMethods, QueryDsl, SelectableHelper};
    use diesel_async::RunQueryDsl;

    // DAO only returns 1 event, but we need 2 for pause detection.
    let last = events::table
        .order(events::timestamp.desc())
        .select(EventRow::as_select())
        .limit(2)
        .load::<EventRow>(conn)
        .await?;

    let mut iter = last.into_iter();
    match iter.next() {
        Some(ev) if ev.event_type == i32::from(EventType::Focus) => {
            let app = ev.app_class.unwrap_or_default();
            let paused = iter
                .next()
                .map(|e| e.event_type == i32::from(EventType::Idle))
                .unwrap_or(false);
            Ok(Some((app, ev.timestamp, paused)))
        }
        _ => Ok(None),
    }
}
