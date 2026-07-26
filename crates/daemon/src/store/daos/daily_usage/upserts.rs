//! Batch-upsert helpers for the daily_usage materialized tables.
//!
//! Every function here takes `conn: &mut DbConn` — they are associated
//! functions called from the delta-computation pipeline.

use diesel::NullableExpressionMethods;
use diesel::dsl::insert_into;
use diesel::{ExpressionMethods, QueryDsl};
use diesel_async::RunQueryDsl;
use wellbeing_core::{Uid, WindowTitle};

use crate::store::connection::DbConn;
use crate::store::schema;

// ── Per-app upserts ─────────────────────────────────────────────────────

pub(crate) async fn upsert_closed_delta(
    conn: &mut DbConn,
    date: &str,
    uid: Uid,
    app_id: i32,
    delta_ms: i64,
) -> anyhow::Result<()> {
    use schema::daily_usage_by_app::columns as c;
    insert_into(schema::daily_usage_by_app::table)
        .values((
            c::date.eq(date),
            c::user_id.eq(uid.0 as i32),
            c::app_id.eq(app_id),
            c::closed_millis.eq(delta_ms),
            c::open_millis.eq(0),
        ))
        .on_conflict((c::date, c::user_id, c::app_id))
        .do_update()
        .set(c::closed_millis.eq(c::closed_millis + delta_ms))
        .execute(conn)
        .await?;
    Ok(())
}

pub(crate) async fn upsert_open_delta(
    conn: &mut DbConn,
    date: &str,
    uid: Uid,
    app_id: i32,
    open_ms: i64,
) -> anyhow::Result<()> {
    use schema::daily_usage_by_app::columns as c;
    insert_into(schema::daily_usage_by_app::table)
        .values((
            c::date.eq(date),
            c::user_id.eq(uid.0 as i32),
            c::app_id.eq(app_id),
            c::closed_millis.eq(0),
            c::open_millis.eq(open_ms),
        ))
        .on_conflict((c::date, c::user_id, c::app_id))
        .do_update()
        .set(c::open_millis.eq(open_ms))
        .execute(conn)
        .await?;
    Ok(())
}

// ── Per-title upserts ────────────────────────────────────────────────────

pub(crate) async fn upsert_closed_delta_by_title(
    conn: &mut DbConn,
    date: &str,
    uid: Uid,
    app_class: &str,
    title: &WindowTitle,
    delta_ms: i64,
) -> anyhow::Result<()> {
    use schema::apps;
    use schema::daily_usage_by_title::columns as c;

    let app_classs_subq = apps::table
        .filter(apps::app_class.eq(app_class))
        .select(apps::id)
        .single_value()
        .assume_not_null();

    insert_into(schema::daily_usage_by_title::table)
        .values((
            c::date.eq(date),
            c::user_id.eq(uid.0 as i32),
            c::app_id.eq(app_classs_subq),
            c::title.eq(title.as_str()),
            c::closed_millis.eq(delta_ms),
            c::open_millis.eq(0),
        ))
        .on_conflict((c::date, c::user_id, c::app_id, c::title))
        .do_update()
        .set(c::closed_millis.eq(c::closed_millis + delta_ms))
        .execute(conn)
        .await?;
    Ok(())
}

pub(crate) async fn upsert_open_delta_by_title(
    conn: &mut DbConn,
    date: &str,
    uid: Uid,
    app_class: &str,
    title: &WindowTitle,
    open_ms: i64,
) -> anyhow::Result<()> {
    use schema::apps;
    use schema::daily_usage_by_title::columns as c;

    let app_classs_subq = apps::table
        .filter(apps::app_class.eq(app_class))
        .select(apps::id)
        .single_value()
        .assume_not_null();

    insert_into(schema::daily_usage_by_title::table)
        .values((
            c::date.eq(date),
            c::user_id.eq(uid.0 as i32),
            c::app_id.eq(app_classs_subq),
            c::title.eq(title.as_str()),
            c::closed_millis.eq(0),
            c::open_millis.eq(open_ms),
        ))
        .on_conflict((c::date, c::user_id, c::app_id, c::title))
        .do_update()
        .set(c::open_millis.eq(open_ms))
        .execute(conn)
        .await?;
    Ok(())
}

// ── Per-category upserts ─────────────────────────────────────────────────

pub(crate) async fn upsert_closed_delta_by_category(
    conn: &mut DbConn,
    date: &str,
    uid: Uid,
    category_id: i32,
    delta_ms: i64,
) -> anyhow::Result<()> {
    use schema::daily_usage_by_category::columns as c;
    insert_into(schema::daily_usage_by_category::table)
        .values((
            c::date.eq(date),
            c::user_id.eq(uid.0 as i32),
            c::category_id.eq(category_id),
            c::closed_millis.eq(delta_ms),
            c::open_millis.eq(0),
        ))
        .on_conflict((c::date, c::user_id, c::category_id))
        .do_update()
        .set(c::closed_millis.eq(c::closed_millis + delta_ms))
        .execute(conn)
        .await?;
    Ok(())
}

pub(crate) async fn upsert_open_delta_by_category(
    conn: &mut DbConn,
    date: &str,
    uid: Uid,
    category_id: i32,
    open_ms: i64,
) -> anyhow::Result<()> {
    use schema::daily_usage_by_category::columns as c;
    insert_into(schema::daily_usage_by_category::table)
        .values((
            c::date.eq(date),
            c::user_id.eq(uid.0 as i32),
            c::category_id.eq(category_id),
            c::closed_millis.eq(0),
            c::open_millis.eq(open_ms),
        ))
        .on_conflict((c::date, c::user_id, c::category_id))
        .do_update()
        .set(c::open_millis.eq(open_ms))
        .execute(conn)
        .await?;
    Ok(())
}
