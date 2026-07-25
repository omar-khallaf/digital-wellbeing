//! Daily usage query helpers for D-Bus interface.

use std::collections::HashMap;

use diesel::{ExpressionMethods, QueryDsl};
use diesel_async::RunQueryDsl;
use wellbeing_core::{
    DailySummary, DailyUsageByAppEntry, DailyUsageByTitleEntry, DailyUsageByTitleSummary,
};

use crate::store::DbPool;
use crate::store::schema::{daily_usage, daily_usage_by_title};

fn daily_usage_row_to_entry(r: crate::policy::DailyUsageRow) -> DailyUsageByAppEntry {
    DailyUsageByAppEntry {
        date: r.date,
        user_id: r.user_id as u32,
        app_id: r.app_id,
        total_millis: r.closed_millis + r.open_millis,
    }
}

pub(crate) async fn get_daily_usage(
    pool: &DbPool,
    date: &str,
    uid: u32,
) -> anyhow::Result<Vec<DailyUsageByAppEntry>> {
    let mut conn = pool.get().await?;

    let rows: Vec<crate::policy::DailyUsageRow> = daily_usage::table
        .filter(daily_usage::date.eq(date))
        .filter(daily_usage::user_id.eq(uid as i32))
        .select((
            daily_usage::date,
            daily_usage::user_id,
            daily_usage::app_id,
            daily_usage::closed_millis,
            daily_usage::open_millis,
        ))
        .load(&mut conn)
        .await?;

    Ok(rows.into_iter().map(daily_usage_row_to_entry).collect())
}

pub(crate) async fn get_daily_usage_by_title(
    pool: &DbPool,
    date: &str,
    uid: u32,
) -> anyhow::Result<Vec<DailyUsageByTitleEntry>> {
    let mut conn = pool.get().await?;

    let rows: Vec<(String, i32, String, String, i64, i64)> = daily_usage_by_title::table
        .filter(daily_usage_by_title::date.eq(date))
        .filter(daily_usage_by_title::user_id.eq(uid as i32))
        .select((
            daily_usage_by_title::date,
            daily_usage_by_title::user_id,
            daily_usage_by_title::app_id,
            daily_usage_by_title::title,
            daily_usage_by_title::closed_millis,
            daily_usage_by_title::open_millis,
        ))
        .load(&mut conn)
        .await?;

    Ok(rows
        .into_iter()
        .map(
            |(date, user_id, app_id, title, closed, open)| DailyUsageByTitleEntry {
                date,
                user_id: user_id as u32,
                app_id,
                title,
                total_millis: closed + open,
            },
        )
        .collect())
}

pub(crate) async fn get_usage_range_by_title(
    pool: &DbPool,
    start_date: &str,
    end_date: &str,
    uid: u32,
) -> anyhow::Result<Vec<DailyUsageByTitleSummary>> {
    let mut conn = pool.get().await?;

    let rows: Vec<(String, i32, String, String, i64, i64)> = daily_usage_by_title::table
        .filter(daily_usage_by_title::date.ge(start_date))
        .filter(daily_usage_by_title::date.le(end_date))
        .filter(daily_usage_by_title::user_id.eq(uid as i32))
        .select((
            daily_usage_by_title::date,
            daily_usage_by_title::user_id,
            daily_usage_by_title::app_id,
            daily_usage_by_title::title,
            daily_usage_by_title::closed_millis,
            daily_usage_by_title::open_millis,
        ))
        .load(&mut conn)
        .await?;

    let mut summaries: Vec<DailyUsageByTitleSummary> = Vec::new();
    for (date, user_id, app_id, title, closed, open) in rows {
        let entry = DailyUsageByTitleEntry {
            date: date.clone(),
            user_id: user_id as u32,
            app_id,
            title,
            total_millis: closed + open,
        };

        if let Some(s) = summaries.last_mut()
            && s.date == date
        {
            s.entries.push(entry);
            continue;
        }

        summaries.push(DailyUsageByTitleSummary {
            date,
            user_id: user_id as u32,
            entries: vec![entry],
        });
    }

    Ok(summaries)
}

pub(crate) async fn get_usage_range(
    pool: &DbPool,
    start_date: &str,
    end_date: &str,
    uid: u32,
) -> anyhow::Result<Vec<DailySummary>> {
    let mut conn = pool.get().await?;

    let rows: Vec<crate::policy::DailyUsageRow> = daily_usage::table
        .filter(daily_usage::date.ge(start_date))
        .filter(daily_usage::date.le(end_date))
        .filter(daily_usage::user_id.eq(uid as i32))
        .select((
            daily_usage::date,
            daily_usage::user_id,
            daily_usage::app_id,
            daily_usage::closed_millis,
            daily_usage::open_millis,
        ))
        .load(&mut conn)
        .await?;

    let mut grouped: HashMap<String, Vec<DailyUsageByAppEntry>> = HashMap::new();
    for r in rows {
        grouped
            .entry(r.date.clone())
            .or_default()
            .push(daily_usage_row_to_entry(r));
    }

    let mut summaries: Vec<DailySummary> = grouped
        .into_iter()
        .map(|(date, entries)| DailySummary {
            date,
            user_id: entries.as_slice().first().map(|e| e.user_id).unwrap_or(uid),
            entries,
        })
        .collect();

    summaries.sort_by(|a, b| a.date.cmp(&b.date));
    Ok(summaries)
}
