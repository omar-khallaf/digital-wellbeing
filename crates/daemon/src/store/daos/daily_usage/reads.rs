//! Aggregated read queries for the daily_usage materialized tables.
//!
//! DAO functions only — return raw tuples via GROUP BY, no domain type
//! conversion.  Feature repositories map results to domain types.
//!
//! All queries are aggregate-only — per-row, per-date queries were removed
//! when the GUI switched to pre-aggregated summaries over D-Bus.

use diesel::{ExpressionMethods, QueryDsl};
use diesel_async::RunQueryDsl;

use crate::store::connection::DbConn;
use crate::store::schema;

/// (app_class, total_millis) — aggregated per-app total across a date range.
pub type AppSummaryRow = (String, i64);

/// (app_class, title, total_millis) — aggregated per-title total across a date range.
pub type TitleSummaryRow = (String, String, i64);

/// Get today's total usage (closed + open) for a specific app.
pub(crate) async fn get_today_app_usage(
    conn: &mut DbConn,
    app_id: i32,
    uid: wellbeing_core::Uid,
    today: &str,
) -> anyhow::Result<i64> {
    use schema::daily_usage_by_app::columns as c;
    let row: Option<(i64, i64)> = schema::daily_usage_by_app::table
        .filter(c::date.eq(today))
        .filter(c::user_id.eq(uid.0 as i32))
        .filter(c::app_id.eq(app_id))
        .select((c::closed_millis, c::open_millis))
        .first(conn)
        .await
        .ok();
    let (closed, open) = row.unwrap_or((0, 0));
    Ok(closed + open)
}

/// Get per-app usage aggregated across a date range, sorted by total_millis DESC.
///
/// Returns one row per app with the sum of (closed_millis + open_millis) across
/// all dates.  The daemon's `ReportsRepo` maps these to `AppUsageSummary`.
pub(crate) async fn app_usage_summary(
    conn: &mut DbConn,
    start_date: &str,
    end_date: &str,
    uid: u32,
) -> anyhow::Result<Vec<AppSummaryRow>> {
    use diesel::dsl::sql;
    use diesel::sql_types::BigInt;
    use schema::apps;
    use schema::daily_usage_by_app::columns as d;

    Ok(schema::daily_usage_by_app::table
        .inner_join(apps::table)
        .filter(d::date.ge(start_date))
        .filter(d::date.le(end_date))
        .filter(d::user_id.eq(uid as i32))
        .group_by(apps::app_class)
        .select((apps::app_class, sql::<BigInt>("SUM(total_millis)")))
        .order(sql::<BigInt>("SUM(total_millis)").desc())
        .load(conn)
        .await?)
}

/// Get per-title usage aggregated across a date range, sorted by total_millis DESC.
///
/// Returns one row per (app_class, title) with the sum of
/// (closed_millis + open_millis) across all dates.
pub(crate) async fn title_usage_summary(
    conn: &mut DbConn,
    start_date: &str,
    end_date: &str,
    uid: u32,
) -> anyhow::Result<Vec<TitleSummaryRow>> {
    use diesel::dsl::sql;
    use diesel::sql_types::BigInt;
    use schema::apps;
    use schema::daily_usage_by_title::columns as d;

    Ok(schema::daily_usage_by_title::table
        .inner_join(apps::table)
        .filter(d::date.ge(start_date))
        .filter(d::date.le(end_date))
        .filter(d::user_id.eq(uid as i32))
        .group_by((apps::app_class, d::title))
        .select((
            apps::app_class,
            d::title,
            sql::<BigInt>("SUM(total_millis)"),
        ))
        .order(sql::<BigInt>("SUM(total_millis)").desc())
        .load(conn)
        .await?)
}

/// Get per-category usage aggregated across a date range, sorted by total_millis DESC.
///
/// Returns one row per category with the sum of (closed_millis + open_millis)
/// across all dates.
pub(crate) async fn category_usage_summary(
    conn: &mut DbConn,
    start_date: &str,
    end_date: &str,
    uid: u32,
) -> anyhow::Result<Vec<(String, i64)>> {
    use diesel::dsl::sql;
    use diesel::sql_types::BigInt;
    use schema::categories;
    use schema::daily_usage_by_category::columns as d;

    Ok(schema::daily_usage_by_category::table
        .inner_join(categories::table)
        .filter(d::date.ge(start_date))
        .filter(d::date.le(end_date))
        .filter(d::user_id.eq(uid as i32))
        .group_by(categories::name)
        .select((categories::name, sql::<BigInt>("SUM(total_millis)")))
        .order(sql::<BigInt>("SUM(total_millis)").desc())
        .load(conn)
        .await?)
}

/// Get total usage per date across a range, sorted by date ASC.
///
/// Returns one row per date with the sum of total_millis across all apps.
/// The GUI bar chart uses this directly — no more flattening and summing
/// per-entry data in memory.
pub(crate) async fn daily_bar_totals(
    conn: &mut DbConn,
    start_date: &str,
    end_date: &str,
    uid: u32,
) -> anyhow::Result<Vec<(String, i64)>> {
    use diesel::dsl::sql;
    use diesel::sql_types::BigInt;
    use schema::daily_usage_by_app::columns as d;

    Ok(schema::daily_usage_by_app::table
        .filter(d::date.ge(start_date))
        .filter(d::date.le(end_date))
        .filter(d::user_id.eq(uid as i32))
        .group_by(d::date)
        .select((d::date, sql::<BigInt>("SUM(total_millis)")))
        .order(d::date.asc())
        .load(conn)
        .await?)
}
