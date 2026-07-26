//! Read queries for the daily_usage materialized tables.
//!
//! DAO functions only — return raw tuples, no domain type conversion.
//! Feature repositories apply `TryFrom`/`TryInto` to map to domain types.

use diesel::{ExpressionMethods, QueryDsl};
use diesel_async::RunQueryDsl;
use wellbeing_core::Uid;

use crate::store::connection::DbConn;
use crate::store::schema;

// ── Raw row types (public so feature repos can impl TryFrom) ───────────

/// (date, user_id, app_class, closed_millis, open_millis) — from daily_usage_by_app ⋈ apps.
pub type UsageAppRow = (String, i32, String, i64, i64);

/// (date, user_id, app_class, title, closed_millis, open_millis) — from daily_usage_by_title ⋈ apps.
pub type UsageTitleRow = (String, i32, String, String, i64, i64);

/// (date, user_id, category_name, closed_millis, open_millis) — from daily_usage_by_category ⋈ categories.
pub type UsageCategoryRow = (String, i32, String, i64, i64);

// ── Per-app reads ───────────────────────────────────────────────────────

/// Get per-app usage for a single date.
pub(crate) async fn daily_usage_by_app(
    conn: &mut DbConn,
    date: &str,
    uid: u32,
) -> anyhow::Result<Vec<UsageAppRow>> {
    use schema::apps;
    use schema::daily_usage_by_app::columns as d;

    Ok(schema::daily_usage_by_app::table
        .inner_join(apps::table)
        .filter(d::date.eq(date))
        .filter(d::user_id.eq(uid as i32))
        .select((
            d::date,
            d::user_id,
            apps::app_class,
            d::closed_millis,
            d::open_millis,
        ))
        .load(conn)
        .await?)
}

/// Get today's total usage (closed + open) for a specific app.
pub(crate) async fn get_today_app_usage(
    conn: &mut DbConn,
    app_id: i32,
    uid: Uid,
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

/// Get per-app usage for a date range.
pub(crate) async fn usage_range_by_app(
    conn: &mut DbConn,
    start_date: &str,
    end_date: &str,
    uid: u32,
) -> anyhow::Result<Vec<UsageAppRow>> {
    use schema::apps;
    use schema::daily_usage_by_app::columns as d;

    Ok(schema::daily_usage_by_app::table
        .inner_join(apps::table)
        .filter(d::date.ge(start_date))
        .filter(d::date.le(end_date))
        .filter(d::user_id.eq(uid as i32))
        .select((
            d::date,
            d::user_id,
            apps::app_class,
            d::closed_millis,
            d::open_millis,
        ))
        .load(conn)
        .await?)
}

// ── Per-title reads ────────────────────────────────────────────────────

/// Get per-title usage for a single date.
pub(crate) async fn daily_usage_by_title(
    conn: &mut DbConn,
    date: &str,
    uid: u32,
) -> anyhow::Result<Vec<UsageTitleRow>> {
    use schema::apps;
    use schema::daily_usage_by_title::columns as d;

    Ok(schema::daily_usage_by_title::table
        .inner_join(apps::table)
        .filter(d::date.eq(date))
        .filter(d::user_id.eq(uid as i32))
        .select((
            d::date,
            d::user_id,
            apps::app_class,
            d::title,
            d::closed_millis,
            d::open_millis,
        ))
        .load(conn)
        .await?)
}

/// Get per-title usage for a date range (raw entries, not grouped).
pub(crate) async fn usage_range_by_title(
    conn: &mut DbConn,
    start_date: &str,
    end_date: &str,
    uid: u32,
) -> anyhow::Result<Vec<UsageTitleRow>> {
    use schema::apps;
    use schema::daily_usage_by_title::columns as d;

    Ok(schema::daily_usage_by_title::table
        .inner_join(apps::table)
        .filter(d::date.ge(start_date))
        .filter(d::date.le(end_date))
        .filter(d::user_id.eq(uid as i32))
        .select((
            d::date,
            d::user_id,
            apps::app_class,
            d::title,
            d::closed_millis,
            d::open_millis,
        ))
        .load(conn)
        .await?)
}

// ── Per-category reads ─────────────────────────────────────────────────

/// Get per-category usage for a date range.
pub(crate) async fn usage_range_by_category(
    conn: &mut DbConn,
    start_date: &str,
    end_date: &str,
    uid: u32,
) -> anyhow::Result<Vec<UsageCategoryRow>> {
    use schema::categories;
    use schema::daily_usage_by_category::columns as d;

    Ok(schema::daily_usage_by_category::table
        .inner_join(categories::table)
        .filter(d::date.ge(start_date))
        .filter(d::date.le(end_date))
        .filter(d::user_id.eq(uid as i32))
        .select((
            d::date,
            d::user_id,
            categories::name,
            d::closed_millis,
            d::open_millis,
        ))
        .load(conn)
        .await?)
}
