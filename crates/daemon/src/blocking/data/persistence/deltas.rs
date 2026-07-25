//! Delta/interval computation functions for daily usage materialization.
//!
//! Pairs `WindowFocused` open events with close events to produce
//! closed-interval durations, written to both `daily_usage` (per-app)
//! and `daily_usage_by_title` (per-title). Open intervals at end of
//! buffer are tracked for minute-tick accumulation via `increment_open_ms`.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use diesel::QueryResult;
use diesel::{ExpressionMethods, QueryDsl};
use diesel_async::{AsyncConnection, RunQueryDsl};
use wellbeing_core::{
    AppId, Uid, WindowTitle,
    event_types::{EVENT_WINDOW_FOCUSED, is_close_event_type},
};

use crate::store::schema;

use super::super::super::buffer::BufferedEvent;
use super::events::BlockingRepo;

impl BlockingRepo {
    /// Apply closed-interval deltas from a buffered event batch.
    ///
    /// Walks buffered events chronologically, tracking open focus intervals
    /// per `uid`. When a new `WindowFocused` arrives for a uid that already
    /// has an open interval, that interval is implicitly closed at the new
    /// focus timestamp — no synthetic `Unfocused` event is needed.
    ///
    /// Closed intervals write to both `daily_usage` (per-app) and
    /// `daily_usage_by_title` (per-title). Open intervals at end of buffer
    /// get a placeholder row for `increment_open_ms` to accumulate later.
    pub async fn apply_closed_deltas_from_buffer<Conn>(
        &self,
        conn: &mut Conn,
        events: &[BufferedEvent],
        now: DateTime<Utc>,
    ) -> anyhow::Result<()>
    where
        Conn: AsyncConnection<Backend = diesel::sqlite::Sqlite>,
    {
        if events.is_empty() {
            return Ok(());
        }

        let today_start_millis = now
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp_millis();
        let now_millis = now.timestamp_millis();

        let mut open_focus: HashMap<Uid, &BufferedEvent> = HashMap::new();

        for event in events {
            if is_close_event_type(event.event_type) {
                let uid = event.uid;
                if let Some(focus) = open_focus.remove(&uid) {
                    Self::upsert_closed_delta_for_pair(
                        conn,
                        uid,
                        &focus.app_id,
                        &focus.timestamp,
                        &event.timestamp,
                        focus.title.as_ref(),
                    )
                    .await?;
                } else {
                    Self::apply_pre_buffer_close(conn, uid, event, today_start_millis, now_millis)
                        .await?;
                }
            } else if event.event_type == EVENT_WINDOW_FOCUSED {
                // Implicitly close the previous interval for this uid.
                // No synthetic Unfocused is needed — the new focus event
                // naturally terminates the prior interval.
                if let Some(prev) = open_focus.insert(event.uid, event) {
                    Self::upsert_closed_delta_for_pair(
                        conn,
                        event.uid,
                        &prev.app_id,
                        &prev.timestamp,
                        &event.timestamp,
                        prev.title.as_ref(),
                    )
                    .await?;
                }
            }
        }

        // Open intervals at end of buffer → ensure a daily_usage row exists
        // so minute-tick `increment_open_ms` can accumulate into it.
        for (uid, focus) in open_focus {
            let date = focus.timestamp.format("%Y-%m-%d").to_string();
            Self::ensure_row_for_open(conn, &date, uid, &focus.app_id).await?;
            if let Some(title) = &focus.title {
                Self::ensure_row_for_open_by_title(conn, &date, uid, &focus.app_id, title).await?;
            }
        }

        Ok(())
    }

    /// Accumulate open-interval time for the current minute-tick.
    ///
    /// Reads the last WindowFocused event for the user today to find
    /// the open-interval start, then sets `open_millis` to that
    /// elapsed duration. This is O(1) per active uid: one indexed
    /// read, one upsert. No event-table scan.
    pub async fn increment_open_ms<Conn>(
        &self,
        conn: &mut Conn,
        uid: Uid,
        app_id: AppId,
        now: DateTime<Utc>,
    ) -> anyhow::Result<()>
    where
        Conn: AsyncConnection<Backend = diesel::sqlite::Sqlite>,
    {
        let today = now.format("%Y-%m-%d").to_string();
        let today_start_millis = now
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp_millis();
        let now_millis = now.timestamp_millis();

        let (open_ms, title): (i64, Option<String>) = match schema::events::table
            .filter(schema::events::user_id.eq(uid.0 as i32))
            .filter(schema::events::event_type.eq(EVENT_WINDOW_FOCUSED))
            .filter(schema::events::app_id.eq(app_id.as_ref()))
            .filter(schema::events::timestamp.ge(today_start_millis))
            .filter(schema::events::timestamp.le(now_millis))
            .order(schema::events::timestamp.desc())
            .limit(1)
            .select((schema::events::timestamp, schema::events::title))
            .first::<(i64, Option<String>)>(conn)
            .await
        {
            Ok((ts, t)) => (Self::duration_millis(ts, now_millis), t),
            Err(diesel::result::Error::NotFound) => (0, None),
            Err(e) => return Err(e.into()),
        };

        let title = title.unwrap_or_else(|| "(untitled)".to_string());

        diesel::insert_into(schema::daily_usage::table)
            .values((
                schema::daily_usage::date.eq(&today),
                schema::daily_usage::user_id.eq(uid.0 as i32),
                schema::daily_usage::app_id.eq(app_id.as_ref()),
                schema::daily_usage::closed_millis.eq(0),
                schema::daily_usage::open_millis.eq(open_ms as i32),
            ))
            .on_conflict((
                schema::daily_usage::date,
                schema::daily_usage::user_id,
                schema::daily_usage::app_id,
            ))
            .do_update()
            .set(schema::daily_usage::open_millis.eq(open_ms as i32))
            .execute(conn)
            .await?;

        diesel::insert_into(schema::daily_usage_by_title::table)
            .values((
                schema::daily_usage_by_title::date.eq(&today),
                schema::daily_usage_by_title::user_id.eq(uid.0 as i32),
                schema::daily_usage_by_title::app_id.eq(app_id.as_ref()),
                schema::daily_usage_by_title::title.eq(&title),
                schema::daily_usage_by_title::closed_millis.eq(0),
                schema::daily_usage_by_title::open_millis.eq(open_ms as i32),
            ))
            .on_conflict((
                schema::daily_usage_by_title::date,
                schema::daily_usage_by_title::user_id,
                schema::daily_usage_by_title::app_id,
                schema::daily_usage_by_title::title,
            ))
            .do_update()
            .set(schema::daily_usage_by_title::open_millis.eq(open_ms as i32))
            .execute(conn)
            .await?;

        Ok(())
    }

    // ── helpers ────────────────────────────────────────────────────

    /// Upsert a closed-interval delta into daily_usage: add to closed_millis.
    async fn upsert_closed_delta<Conn>(
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
                schema::daily_usage::closed_millis.eq(delta_ms as i32),
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
                    .eq(schema::daily_usage::closed_millis + delta_ms as i32),
                schema::daily_usage::open_millis.eq(0),
            ))
            .execute(conn)
            .await
    }

    /// Upsert a closed-interval delta into daily_usage_by_title.
    async fn upsert_closed_delta_by_title<Conn>(
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
                schema::daily_usage_by_title::closed_millis.eq(delta_ms as i32),
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
                    .eq(schema::daily_usage_by_title::closed_millis + delta_ms as i32),
                schema::daily_usage_by_title::open_millis.eq(0),
            ))
            .execute(conn)
            .await
    }

    /// Ensure a daily_usage row exists for an open interval with open_millis = 0.
    async fn ensure_row_for_open<Conn>(
        conn: &mut Conn,
        today: &str,
        uid: Uid,
        app_id: &AppId,
    ) -> QueryResult<usize>
    where
        Conn: AsyncConnection<Backend = diesel::sqlite::Sqlite>,
    {
        let exists: Option<i32> = schema::daily_usage::table
            .filter(schema::daily_usage::date.eq(today))
            .filter(schema::daily_usage::user_id.eq(uid.0 as i32))
            .filter(schema::daily_usage::app_id.eq(app_id.as_ref()))
            .select(schema::daily_usage::open_millis)
            .first(conn)
            .await
            .ok();

        if exists.is_some() {
            return Ok(0);
        }

        diesel::insert_into(schema::daily_usage::table)
            .values((
                schema::daily_usage::date.eq(today),
                schema::daily_usage::user_id.eq(uid.0 as i32),
                schema::daily_usage::app_id.eq(app_id.as_ref()),
                schema::daily_usage::closed_millis.eq(0),
                schema::daily_usage::open_millis.eq(0),
            ))
            .execute(conn)
            .await
    }

    /// Ensure a daily_usage_by_title row exists for an open interval.
    async fn ensure_row_for_open_by_title<Conn>(
        conn: &mut Conn,
        today: &str,
        uid: Uid,
        app_id: &AppId,
        title: &WindowTitle,
    ) -> QueryResult<usize>
    where
        Conn: AsyncConnection<Backend = diesel::sqlite::Sqlite>,
    {
        let exists: Option<i32> = schema::daily_usage_by_title::table
            .filter(schema::daily_usage_by_title::date.eq(today))
            .filter(schema::daily_usage_by_title::user_id.eq(uid.0 as i32))
            .filter(schema::daily_usage_by_title::app_id.eq(app_id.as_ref()))
            .filter(schema::daily_usage_by_title::title.eq(title.as_str()))
            .select(schema::daily_usage_by_title::open_millis)
            .first(conn)
            .await
            .ok();

        if exists.is_some() {
            return Ok(0);
        }

        diesel::insert_into(schema::daily_usage_by_title::table)
            .values((
                schema::daily_usage_by_title::date.eq(today),
                schema::daily_usage_by_title::user_id.eq(uid.0 as i32),
                schema::daily_usage_by_title::app_id.eq(app_id.as_ref()),
                schema::daily_usage_by_title::title.eq(title.as_str()),
                schema::daily_usage_by_title::closed_millis.eq(0),
                schema::daily_usage_by_title::open_millis.eq(0),
            ))
            .execute(conn)
            .await
    }

    pub(crate) fn duration_millis(start: i64, end: i64) -> i64 {
        (end - start).max(0)
    }

    /// Upsert a closed interval, splitting across UTC-day boundaries.
    async fn upsert_closed_delta_for_pair<Conn>(
        conn: &mut Conn,
        uid: Uid,
        app_id: &AppId,
        focus_ts: &DateTime<Utc>,
        close_ts: &DateTime<Utc>,
        title: Option<&WindowTitle>,
    ) -> anyhow::Result<()>
    where
        Conn: AsyncConnection<Backend = diesel::sqlite::Sqlite>,
    {
        Self::upsert_interval_split_days(conn, uid, app_id, focus_ts, close_ts, title).await
    }

    /// Split a focus interval across UTC-day boundaries and upsert each day
    /// segment into both `daily_usage` and `daily_usage_by_title`.
    async fn upsert_interval_split_days<Conn>(
        conn: &mut Conn,
        uid: Uid,
        app_id: &AppId,
        focus_ts: &DateTime<Utc>,
        close_ts: &DateTime<Utc>,
        title: Option<&WindowTitle>,
    ) -> anyhow::Result<()>
    where
        Conn: AsyncConnection<Backend = diesel::sqlite::Sqlite>,
    {
        let mut seg_start = *focus_ts;

        loop {
            let next_boundary = (seg_start.date_naive() + chrono::TimeDelta::days(1))
                .and_hms_opt(0, 0, 0)
                .unwrap()
                .and_utc();

            if next_boundary >= *close_ts {
                let dur = Self::duration_millis(
                    seg_start.timestamp_millis(),
                    close_ts.timestamp_millis(),
                );
                let date = seg_start.format("%Y-%m-%d").to_string();
                Self::upsert_closed_delta(conn, &date, uid, app_id, dur).await?;
                if let Some(t) = title {
                    Self::upsert_closed_delta_by_title(conn, &date, uid, app_id, t, dur).await?;
                }
                break;
            }

            let dur = Self::duration_millis(
                seg_start.timestamp_millis(),
                next_boundary.timestamp_millis(),
            );
            let date = seg_start.format("%Y-%m-%d").to_string();
            Self::upsert_closed_delta(conn, &date, uid, app_id, dur).await?;
            if let Some(t) = title {
                Self::upsert_closed_delta_by_title(conn, &date, uid, app_id, t, dur).await?;
            }
            seg_start = next_boundary;
        }

        Ok(())
    }

    /// Query the last WindowFocused event before the buffer for a uid.
    /// Returns `(timestamp_millis, app_id, title)`.
    async fn resolve_pre_buffer_focus<Conn>(
        conn: &mut Conn,
        uid: Uid,
        today_start_millis: i64,
        now_millis: i64,
    ) -> Option<(i64, Option<String>, Option<String>)>
    where
        Conn: AsyncConnection<Backend = diesel::sqlite::Sqlite>,
    {
        schema::events::table
            .filter(schema::events::user_id.eq(uid.0 as i32))
            .filter(schema::events::event_type.eq(EVENT_WINDOW_FOCUSED))
            .filter(schema::events::timestamp.ge(today_start_millis))
            .filter(schema::events::timestamp.le(now_millis))
            .order(schema::events::timestamp.desc())
            .limit(1)
            .select((
                schema::events::timestamp,
                schema::events::app_id,
                schema::events::title,
            ))
            .first::<(i64, Option<String>, Option<String>)>(conn)
            .await
            .ok()
    }

    /// Handle a close event whose interval started before the buffer.
    async fn apply_pre_buffer_close<Conn>(
        conn: &mut Conn,
        uid: Uid,
        event: &BufferedEvent,
        today_start_millis: i64,
        now_millis: i64,
    ) -> anyhow::Result<()>
    where
        Conn: AsyncConnection<Backend = diesel::sqlite::Sqlite>,
    {
        let Some((start_ts, db_app_id, db_title)) =
            Self::resolve_pre_buffer_focus(conn, uid, today_start_millis, now_millis).await
        else {
            return Ok(());
        };
        let Some(ref resolved) = db_app_id else {
            return Ok(());
        };
        let valid = AppId::new(resolved).unwrap_or_else(|_| AppId::new("unknown").unwrap());
        let title_str = db_title.as_deref().unwrap_or("(untitled)");
        let title = WindowTitle::new(title_str);
        let title_ref: Option<&WindowTitle> = Some(&title);

        let start_dt = DateTime::from_timestamp_millis(start_ts)
            .ok_or_else(|| anyhow::anyhow!("resolved focus timestamp must be valid: {start_ts}"))?;
        Self::upsert_interval_split_days(conn, uid, &valid, &start_dt, &event.timestamp, title_ref)
            .await?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, Utc};
    use diesel::ExpressionMethods;
    use diesel_async::RunQueryDsl;

    use tempfile::TempDir;

    use wellbeing_core::event_types::EVENT_WINDOW_FOCUSED;
    use wellbeing_core::{AppId, Uid};

    use crate::store::StoreBuilder;

    use super::super::super::super::buffer::BufferedEvent;
    use super::BlockingRepo;

    fn app(s: &str) -> AppId {
        AppId::new(s).unwrap()
    }

    fn dt(ms: i64) -> chrono::DateTime<Utc> {
        DateTime::from_timestamp_millis(ms).unwrap()
    }

    fn focus_event(uid: u32, app_name: &str, ts: i64) -> BufferedEvent {
        BufferedEvent {
            uid: Uid(uid),
            app_id: app(app_name),
            event_type: 0,
            timestamp: dt(ts),
            title: None,
        }
    }

    fn close_event(uid: u32, app_name: &str, event_type: i32, ts: i64) -> BufferedEvent {
        BufferedEvent {
            uid: Uid(uid),
            app_id: app(app_name),
            event_type,
            timestamp: dt(ts),
            title: None,
        }
    }

    /// Create a test database with the initial migration schema.
    /// Uses `StoreBuilder::build` which runs all migrations.
    async fn setup_repo() -> (TempDir, BlockingRepo) {
        let tmp = TempDir::new().expect("temp dir");
        let db_path = tmp.path().join("test.db");

        let pool = StoreBuilder::new(db_path.clone())
            .build()
            .await
            .expect("build store");

        let repo = BlockingRepo::new(pool);
        (tmp, repo)
    }

    /// Helper: write a raw event into the DB (simulating pre-buffer history).
    /// Stores the timestamp in milliseconds (matching production
    /// `flush_events` which uses `.timestamp_millis()`).
    async fn write_raw_event(
        repo: &BlockingRepo,
        event_type: i32,
        uid: u32,
        app_id: &str,
        ts: i64,
    ) {
        let mut conn = repo.pool.get().await.unwrap();
        diesel::insert_into(crate::store::schema::events::table)
            .values((
                crate::store::schema::events::event_type.eq(event_type),
                crate::store::schema::events::user_id.eq(uid as i32),
                crate::store::schema::events::timestamp.eq(ts),
                crate::store::schema::events::app_id.eq(Some(app_id)),
            ))
            .execute(&mut conn)
            .await
            .unwrap();
    }

    async fn read_closed_millis(
        repo: &BlockingRepo,
        date: &str,
        uid: u32,
        app_id: &str,
    ) -> Option<i32> {
        use diesel::QueryDsl;

        let mut conn = repo.pool.get().await.unwrap();
        crate::store::schema::daily_usage::table
            .filter(crate::store::schema::daily_usage::date.eq(date))
            .filter(crate::store::schema::daily_usage::user_id.eq(uid as i32))
            .filter(crate::store::schema::daily_usage::app_id.eq(app_id))
            .select(crate::store::schema::daily_usage::closed_millis)
            .first::<i32>(&mut conn)
            .await
            .ok()
    }

    #[tokio::test]
    async fn test_pre_buffer_focus_is_closed_by_close_event() {
        // Given: a pre-buffer WindowFocused at ts=1000 for uid=1000, app=firefox
        let (_tmp, repo) = setup_repo().await;
        write_raw_event(&repo, EVENT_WINDOW_FOCUSED, 1000, "firefox", 1_000_000).await;

        // When: a close event arrives in the buffer at ts=3000
        let events = vec![close_event(1000, "firefox", 1, 3_000_000)];
        let mut conn = repo.pool.get().await.unwrap();
        repo.apply_closed_deltas_from_buffer(&mut conn, &events, dt(3_000_000))
            .await
            .unwrap();

        // Then: closed_millis = 3_000_000 - 1_000_000 = 2_000_000
        let closed = read_closed_millis(&repo, "1970-01-01", 1000, "firefox").await;
        assert_eq!(closed, Some(2_000_000));
    }

    #[tokio::test]
    async fn test_window_focused_implicitly_closes_previous_interval() {
        // Given: two WindowFocused events in the buffer for the same uid but different apps
        let (_tmp, repo) = setup_repo().await;
        let events = vec![
            focus_event(1000, "firefox", 1_000_000),
            focus_event(1000, "code", 1_000_100),
        ];
        let mut conn = repo.pool.get().await.unwrap();
        repo.apply_closed_deltas_from_buffer(&mut conn, &events, dt(1_000_100))
            .await
            .unwrap();

        // Then: firefox interval (1_000_000 → 1_000_100) = 100ms closed
        let closed = read_closed_millis(&repo, "1970-01-01", 1000, "firefox").await;
        assert_eq!(closed, Some(100));
    }

    #[tokio::test]
    async fn test_window_blocked_closes_previous_interval() {
        // Given: WindowFocused then WindowBlocked for the same uid
        let (_tmp, repo) = setup_repo().await;
        let events = vec![
            focus_event(1000, "firefox", 1_000_000),
            close_event(1000, "firefox", 8, 1_001_000), // EVENT_WINDOW_BLOCKED = 8
        ];
        let mut conn = repo.pool.get().await.unwrap();
        repo.apply_closed_deltas_from_buffer(&mut conn, &events, dt(1_001_000))
            .await
            .unwrap();

        // Then: firefox interval (1_000_000 → 1_001_000) = 1000ms closed
        let closed = read_closed_millis(&repo, "1970-01-01", 1000, "firefox").await;
        assert_eq!(closed, Some(1000));
    }
}
