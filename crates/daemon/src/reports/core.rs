//! Report generation and data aggregation functions.

use std::time::Duration;

use chrono::Duration as ChronoDuration;
use diesel::ExpressionMethods;
use diesel::QueryDsl;
use diesel::SelectableHelper;
use diesel::result::QueryResult;
use diesel_async::RunQueryDsl;
use wellbeing_core::{Clock, DailySummary, DailyUsageEntry, Uid};

use crate::store::DbPool;
use crate::store::connection::DbConn;
use crate::store::schema::{daily_usage, events};

use super::data::models::{DailyUsageRow, EventRow};

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

    Ok(rows
        .into_iter()
        .map(|r| DailyUsageEntry {
            date: r.date,
            user_id: r.user_id as u32,
            app_id: r.app_id,
            total_millis: (r.closed_millis as i64) + (r.open_millis as i64),
            extended: r.extended,
        })
        .collect())
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
        let entry = DailyUsageEntry {
            date: r.date.clone(),
            user_id: r.user_id as u32,
            app_id: r.app_id,
            total_millis: (r.closed_millis as i64) + (r.open_millis as i64),
            extended: r.extended,
        };

        let last = summaries.last_mut();
        if let Some(s) = last
            && s.date == r.date
        {
            s.entries.push(entry);
            continue;
        }

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
        .order(events::id.desc())
        .select(EventRow::as_select())
        .limit(1)
        .load::<EventRow>(conn)
        .await?;
    Ok(items.into_iter().next())
}

/// Get events within a timestamp range.
pub async fn get_event_range(
    conn: &mut DbConn,
    start: &str,
    end: &str,
) -> QueryResult<Vec<EventRow>> {
    events::table
        .filter(events::timestamp.ge(start))
        .filter(events::timestamp.le(end))
        .order(events::timestamp.asc())
        .select(EventRow::as_select())
        .load::<EventRow>(conn)
        .await
}

/// Check if we need to open an interval at startup (crashed with active focus).
pub async fn open_interval_at_startup(
    conn: &mut DbConn,
) -> QueryResult<Option<(String, String, bool)>> {
    let last = events::table
        .order(events::id.desc())
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
    use diesel::sql_types::{Integer, Text};

    let mut conn = pool.get().await?;
    let cutoff = clock.now() - ChronoDuration::days(90);
    let cutoff_dt = cutoff.format("%Y-%m-%dT%H:%M:%SZ").to_string();
    let cutoff_date = cutoff.format("%Y-%m-%d").to_string();

    loop {
        let count = sql_query(
            "DELETE FROM events WHERE id IN (SELECT id FROM events WHERE timestamp < $1 LIMIT $2)",
        )
        .bind::<Text, _>(&cutoff_dt)
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
