//! Standalone helper functions for delta computation.
//!
//! These are `impl BlockingRepo` associated functions (no `&self`) that perform
//! individual upserts against `daily_usage` / `daily_usage_by_title`.
//! They are `pub(super)` so the parent `deltas` module can call them directly
//! via `Self::method_name(...)`.

use diesel::ExpressionMethods;
use diesel::QueryResult;
use diesel_async::{AsyncConnection, RunQueryDsl};
use wellbeing_core::{AppId, Uid, WindowTitle};

use crate::store::schema;

use super::BlockingRepo;

impl BlockingRepo {
    /// Upsert a closed-interval delta into daily_usage: add to closed_millis,
    /// reset open_millis to 0.
    pub(super) async fn upsert_closed_delta<Conn>(
        conn: &mut Conn,
        today: &str,
        uid: Uid,
        app_id: &AppId,
        delta_ms: i64,
    ) -> QueryResult<usize>
    where
        Conn: AsyncConnection<Backend = diesel::sqlite::Sqlite>,
    {
        diesel::insert_into(schema::daily_usage::table)
            .values((
                schema::daily_usage::date.eq(today),
                schema::daily_usage::user_id.eq(uid.0 as i32),
                schema::daily_usage::app_id.eq(app_id.as_ref()),
                schema::daily_usage::closed_millis.eq(delta_ms),
                schema::daily_usage::open_millis.eq(0),
            ))
            .on_conflict((
                schema::daily_usage::date,
                schema::daily_usage::user_id,
                schema::daily_usage::app_id,
            ))
            .do_update()
            .set((
                schema::daily_usage::closed_millis
                    .eq(schema::daily_usage::closed_millis + delta_ms),
                schema::daily_usage::open_millis.eq(0),
            ))
            .execute(conn)
            .await
    }

    /// Upsert a closed-interval delta into daily_usage_by_title.
    pub(super) async fn upsert_closed_delta_by_title<Conn>(
        conn: &mut Conn,
        today: &str,
        uid: Uid,
        app_id: &AppId,
        title: &WindowTitle,
        delta_ms: i64,
    ) -> QueryResult<usize>
    where
        Conn: AsyncConnection<Backend = diesel::sqlite::Sqlite>,
    {
        diesel::insert_into(schema::daily_usage_by_title::table)
            .values((
                schema::daily_usage_by_title::date.eq(today),
                schema::daily_usage_by_title::user_id.eq(uid.0 as i32),
                schema::daily_usage_by_title::app_id.eq(app_id.as_ref()),
                schema::daily_usage_by_title::title.eq(title.as_str()),
                schema::daily_usage_by_title::closed_millis.eq(delta_ms),
                schema::daily_usage_by_title::open_millis.eq(0),
            ))
            .on_conflict((
                schema::daily_usage_by_title::date,
                schema::daily_usage_by_title::user_id,
                schema::daily_usage_by_title::app_id,
                schema::daily_usage_by_title::title,
            ))
            .do_update()
            .set((
                schema::daily_usage_by_title::closed_millis
                    .eq(schema::daily_usage_by_title::closed_millis + delta_ms),
                schema::daily_usage_by_title::open_millis.eq(0),
            ))
            .execute(conn)
            .await
    }

    /// Upsert an open interval into daily_usage: SET open_millis, leave
    /// closed_millis untouched. Creates the row if it doesn't exist.
    pub(super) async fn upsert_open_delta<Conn>(
        conn: &mut Conn,
        today: &str,
        uid: Uid,
        app_id: &AppId,
        open_ms: i64,
    ) -> QueryResult<usize>
    where
        Conn: AsyncConnection<Backend = diesel::sqlite::Sqlite>,
    {
        diesel::insert_into(schema::daily_usage::table)
            .values((
                schema::daily_usage::date.eq(today),
                schema::daily_usage::user_id.eq(uid.0 as i32),
                schema::daily_usage::app_id.eq(app_id.as_ref()),
                schema::daily_usage::closed_millis.eq(0),
                schema::daily_usage::open_millis.eq(open_ms),
            ))
            .on_conflict((
                schema::daily_usage::date,
                schema::daily_usage::user_id,
                schema::daily_usage::app_id,
            ))
            .do_update()
            .set(schema::daily_usage::open_millis.eq(open_ms))
            .execute(conn)
            .await
    }

    /// Upsert an open interval into daily_usage_by_title: SET open_millis.
    pub(super) async fn upsert_open_delta_by_title<Conn>(
        conn: &mut Conn,
        today: &str,
        uid: Uid,
        app_id: &AppId,
        title: &WindowTitle,
        open_ms: i64,
    ) -> QueryResult<usize>
    where
        Conn: AsyncConnection<Backend = diesel::sqlite::Sqlite>,
    {
        diesel::insert_into(schema::daily_usage_by_title::table)
            .values((
                schema::daily_usage_by_title::date.eq(today),
                schema::daily_usage_by_title::user_id.eq(uid.0 as i32),
                schema::daily_usage_by_title::app_id.eq(app_id.as_ref()),
                schema::daily_usage_by_title::title.eq(title.as_str()),
                schema::daily_usage_by_title::closed_millis.eq(0),
                schema::daily_usage_by_title::open_millis.eq(open_ms),
            ))
            .on_conflict((
                schema::daily_usage_by_title::date,
                schema::daily_usage_by_title::user_id,
                schema::daily_usage_by_title::app_id,
                schema::daily_usage_by_title::title,
            ))
            .do_update()
            .set(schema::daily_usage_by_title::open_millis.eq(open_ms))
            .execute(conn)
            .await
    }
}
