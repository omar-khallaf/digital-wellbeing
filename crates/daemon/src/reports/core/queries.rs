//! Individual date-range and point queries against the reports database.

use diesel::ExpressionMethods;
use diesel::QueryDsl;
use diesel::SelectableHelper;
use diesel::result::QueryResult;
use diesel_async::RunQueryDsl;
use wellbeing_core::{
    DailySummary, DailyUsageByAppEntry, DailyUsageByTitleEntry, DailyUsageByTitleSummary, Uid,
};

use crate::reports::data::models::{DailyUsageByTitleRow, DailyUsageRow, EventRow};
use crate::store::connection::DbConn;
use crate::store::schema::{daily_usage, daily_usage_by_title, events};

fn row_to_entry(r: DailyUsageRow) -> DailyUsageByAppEntry {
    DailyUsageByAppEntry {
        date: r.date,
        user_id: r.user_id as u32,
        app_id: r.app_id,
        total_millis: (r.closed_millis as i64) + (r.open_millis as i64),
    }
}

pub async fn get_daily_usage(
    conn: &mut DbConn,
    date: &str,
    user_id: Uid,
) -> QueryResult<Vec<DailyUsageByAppEntry>> {
    let rows: Vec<DailyUsageRow> = daily_usage::table
        .filter(daily_usage::date.eq(date))
        .filter(daily_usage::user_id.eq(user_id.0 as i32))
        .select(DailyUsageRow::as_select())
        .load(conn)
        .await?;

    Ok(rows.into_iter().map(row_to_entry).collect())
}

pub async fn get_daily_usage_by_title(
    conn: &mut DbConn,
    date: &str,
    user_id: Uid,
) -> QueryResult<Vec<DailyUsageByTitleEntry>> {
    let rows: Vec<DailyUsageByTitleRow> = daily_usage_by_title::table
        .filter(daily_usage_by_title::date.eq(date))
        .filter(daily_usage_by_title::user_id.eq(user_id.0 as i32))
        .select(DailyUsageByTitleRow::as_select())
        .load(conn)
        .await?;

    Ok(rows
        .into_iter()
        .map(|r| DailyUsageByTitleEntry {
            date: r.date,
            user_id: r.user_id as u32,
            app_id: r.app_id,
            title: r.title,
            total_millis: (r.closed_millis as i64) + (r.open_millis as i64),
        })
        .collect())
}

pub async fn get_usage_range_by_title(
    conn: &mut DbConn,
    start: &str,
    end: &str,
    user_id: Uid,
) -> QueryResult<Vec<DailyUsageByTitleSummary>> {
    let rows: Vec<DailyUsageByTitleRow> = daily_usage_by_title::table
        .filter(daily_usage_by_title::date.ge(start))
        .filter(daily_usage_by_title::date.le(end))
        .filter(daily_usage_by_title::user_id.eq(user_id.0 as i32))
        .order(daily_usage_by_title::date.asc())
        .select(DailyUsageByTitleRow::as_select())
        .load(conn)
        .await?;

    let mut summaries: Vec<DailyUsageByTitleSummary> = Vec::new();

    for r in rows {
        let entry = DailyUsageByTitleEntry {
            date: r.date.clone(),
            user_id: user_id.0,
            app_id: r.app_id,
            title: r.title,
            total_millis: (r.closed_millis as i64) + (r.open_millis as i64),
        };

        if let Some(s) = summaries.last_mut()
            && s.date == r.date
        {
            s.entries.push(entry);
            continue;
        }

        summaries.push(DailyUsageByTitleSummary {
            date: r.date,
            user_id: user_id.0,
            entries: vec![entry],
        });
    }

    Ok(summaries)
}

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
        let entry = DailyUsageByAppEntry {
            date: r.date.clone(),
            user_id: r.user_id as u32,
            app_id: r.app_id,
            total_millis: (r.closed_millis as i64) + (r.open_millis as i64),
        };
        summaries.push(DailySummary {
            date: r.date,
            user_id: r.user_id as u32,
            entries: vec![entry],
        });
    }

    Ok(summaries)
}

pub async fn last_event(conn: &mut DbConn) -> QueryResult<Option<EventRow>> {
    let items = events::table
        .order(events::timestamp.desc())
        .select(EventRow::as_select())
        .limit(1)
        .load::<EventRow>(conn)
        .await?;
    Ok(items.into_iter().next())
}

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
