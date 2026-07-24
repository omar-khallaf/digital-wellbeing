//! Report generation and data aggregation functions.

use std::time::Duration;

use chrono::{Duration as ChronoDuration, NaiveDate, TimeDelta};
use diesel::ExpressionMethods;
use diesel::QueryDsl;
use diesel::SelectableHelper;
use diesel::result::QueryResult;
use diesel_async::RunQueryDsl;
use wellbeing_core::{Clock, DailySummary, DailyUsageEntry, Uid};

use crate::store::DbPool;
use crate::store::connection::DbConn;
use crate::store::schema::{daily_usage, events};

use super::data::models::{DailyUsageRow, EventRow, HourlyUsageRow};
use crate::blocking::data::CLOSE_EVENT_TYPES;

/// Map a `DailyUsageRow` (DB model) into a `DailyUsageEntry` (domain value).
fn row_to_entry(r: DailyUsageRow) -> DailyUsageEntry {
    DailyUsageEntry {
        date: r.date,
        user_id: r.user_id as u32,
        app_id: r.app_id,
        total_millis: (r.closed_millis as i64) + (r.open_millis as i64),
        extended: r.extended,
    }
}

/// Get daily usage entries for a specific date and user.
pub async fn get_daily_usage(
    conn: &mut DbConn,
    date: &str,
    user_id: Uid,
) -> QueryResult<Vec<DailyUsageEntry>> {
    let rows: Vec<DailyUsageRow> = daily_usage::table
        .filter(daily_usage::date.eq(date))
        .filter(daily_usage::user_id.eq(user_id.0 as i32))
        .select(DailyUsageRow::as_select())
        .load(conn)
        .await?;

    Ok(rows.into_iter().map(row_to_entry).collect())
}

/// Get daily usage grouped by date for a date range.
pub async fn get_usage_range(
    conn: &mut DbConn,
    start: &str,
    end: &str,
    user_id: Uid,
) -> QueryResult<Vec<DailySummary>> {
    let rows: Vec<DailyUsageRow> = daily_usage::table
        .filter(daily_usage::date.ge(start))
        .filter(daily_usage::date.le(end))
        .filter(daily_usage::user_id.eq(user_id.0 as i32))
        .order(daily_usage::date.asc())
        .select(DailyUsageRow::as_select())
        .load(conn)
        .await?;

    let mut summaries: Vec<DailySummary> = Vec::new();

    for r in rows {
        let last = summaries.last_mut();
        if let Some(s) = last
            && s.date == r.date
        {
            s.entries.push(row_to_entry(r));
            continue;
        }

        // Push branch: r.date is needed for both DailySummary and the entry, so clone once.
        let entry = DailyUsageEntry {
            date: r.date.clone(),
            user_id: r.user_id as u32,
            app_id: r.app_id,
            total_millis: (r.closed_millis as i64) + (r.open_millis as i64),
            extended: r.extended,
        };
        summaries.push(DailySummary {
            date: r.date,
            user_id: r.user_id as u32,
            entries: vec![entry],
        });
    }

    Ok(summaries)
}

/// Get the most recent event row.
pub async fn last_event(conn: &mut DbConn) -> QueryResult<Option<EventRow>> {
    let items = events::table
        .order(events::timestamp.desc())
        .select(EventRow::as_select())
        .limit(1)
        .load::<EventRow>(conn)
        .await?;
    Ok(items.into_iter().next())
}

/// Get events within a timestamp range (epoch millis).
pub async fn get_event_range(
    conn: &mut DbConn,
    start: i64,
    end: i64,
) -> QueryResult<Vec<EventRow>> {
    events::table
        .filter(events::timestamp.ge(start))
        .filter(events::timestamp.le(end))
        .order(events::timestamp.asc())
        .select(EventRow::as_select())
        .load::<EventRow>(conn)
        .await
}

/// Get hourly usage breakdown for a specific date and user.
/// Returns exactly 24 rows (hours 0-23), missing hours filled with 0.
pub async fn get_hourly_usage(
    conn: &mut DbConn,
    user_id: Uid,
    date: NaiveDate,
) -> QueryResult<Vec<HourlyUsageRow>> {
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

    let events = get_event_range(conn, start_millis, end_millis).await?;

    let user_events: Vec<&EventRow> = events
        .iter()
        .filter(|e| e.user_id == user_id.0 as i32)
        .collect();

    let mut hourly = [0i32; 24];
    let mut focus_start: Option<i64> = None;

    for event in &user_events {
        if event.event_type == 0 {
            // WindowFocused — record the start of a focus interval
            focus_start = Some(event.timestamp);
        } else if CLOSE_EVENT_TYPES.contains(&event.event_type) {
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
    hourly: &mut [i32; 24],
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
        let millis = (overlap_end - overlap_start).max(0) as i32;
        *slot += millis;
    }
}

/// Check if we need to open an interval at startup (crashed with active focus).
/// Returns `(app_id, timestamp_millis, is_paused)`.
pub async fn open_interval_at_startup(
    conn: &mut DbConn,
) -> QueryResult<Option<(String, i64, bool)>> {
    let last = events::table
        .order(events::timestamp.desc())
        .select(EventRow::as_select())
        .limit(2)
        .load::<EventRow>(conn)
        .await?;

    let mut iter = last.into_iter();
    match iter.next() {
        Some(ev) if ev.event_type == 0 => {
            let app_id = ev.app_id.unwrap_or_default();
            let paused = iter.next().map(|e| e.event_type == 2).unwrap_or(false);
            Ok(Some((app_id, ev.timestamp, paused)))
        }
        _ => Ok(None),
    }
}

/// Periodic prune loop — runs every hour, deletes old events and daily_usage.
pub async fn prune_loop(pool: DbPool, clock: Box<dyn Clock>) {
    let mut interval = tokio::time::interval(Duration::from_secs(3600));
    interval.tick().await;

    loop {
        interval.tick().await;
        if let Err(e) = prune_cycle(&pool, &*clock).await {
            tracing::error!(error = %e, "prune cycle failed");
        }
    }
}

async fn prune_cycle(pool: &DbPool, clock: &dyn Clock) -> anyhow::Result<()> {
    use diesel::sql_query;
    use diesel::sql_types::{BigInt, Integer, Text};

    let mut conn = pool.get().await?;
    let now = clock.now();
    let cutoff_millis = now.timestamp_millis() - 90 * 86400 * 1000;
    let cutoff_date = (now - ChronoDuration::days(90))
        .format("%Y-%m-%d")
        .to_string();

    loop {
        let count = sql_query(
            "DELETE FROM events WHERE id IN (SELECT id FROM events WHERE timestamp < $1 LIMIT $2)",
        )
        .bind::<BigInt, _>(&cutoff_millis)
        .bind::<Integer, _>(500)
        .execute(&mut conn)
        .await?;
        if count < 500 {
            break;
        }
    }

    loop {
        let count = sql_query(
            "DELETE FROM daily_usage WHERE rowid IN (SELECT rowid FROM daily_usage WHERE date < $1 LIMIT $2)"
        )
            .bind::<Text, _>(&cutoff_date)
            .bind::<Integer, _>(500)
            .execute(&mut conn)
            .await?;
        if count < 500 {
            break;
        }
    }

    Ok(())
}
