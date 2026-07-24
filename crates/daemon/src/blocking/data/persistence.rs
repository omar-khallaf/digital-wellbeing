//! Persistence layer for the blocking/enforcement feature.
//!
//! [`BlockingRepo`] owns all database access. Domain logic (policy
//! evaluation, schedule filtering) is NOT in this module — it stays
//! in `core.rs` / `policy::core`.

use chrono::{DateTime, Utc};
use diesel::QueryResult;
use diesel::{ExpressionMethods, QueryDsl};
use diesel_async::{AsyncConnection, RunQueryDsl};
use wellbeing_core::{
    AppId, CategoryId, Clock, Uid,
    event_types::{EVENT_IDLE, EVENT_RESUMED, EVENT_WINDOW_FOCUSED, is_close_event_type},
};

use super::super::buffer::BufferedEvent;

use crate::policy::{DieselPolicyRepo, Policy, PolicyRepo as _};
use crate::store::{DbPool, schema};

/// Repository for blocking-feature persistence operations.
pub(crate) struct BlockingRepo {
    pool: DbPool,
}

impl BlockingRepo {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    pub(crate) fn pool(&self) -> &DbPool {
        &self.pool
    }

    // ── category / policy / usage lookups ──────────────────────────

    /// Resolve category ids for an app (user-specific then fallback).
    pub async fn fetch_categories(
        &self,
        app_id: &AppId,
        uid: Uid,
    ) -> anyhow::Result<Vec<CategoryId>> {
        let mut conn = self.pool.get().await?;
        DieselPolicyRepo
            .resolve_categories_for_app(&mut conn, app_id, uid)
            .await
    }

    /// Resolve policies for an app, returning domain [`Policy`] values.
    /// Caller applies schedule filtering via `filter_policies_by_schedule`.
    pub async fn fetch_policies(
        &self,
        app_id: &AppId,
        categories: &[CategoryId],
        uid: Uid,
    ) -> anyhow::Result<Vec<Policy>> {
        let mut conn = self.pool.get().await?;
        let rows = DieselPolicyRepo
            .resolve_policies_for_app(&mut conn, app_id, categories, uid)
            .await?;
        Ok(rows.into_iter().map(|r| r.into_domain_policy()).collect())
    }

    /// Get today's accumulated usage for an app.
    pub async fn fetch_usage(
        &self,
        app_id: &AppId,
        uid: Uid,
        clock: &dyn Clock,
    ) -> anyhow::Result<(i64, bool)> {
        let mut conn = self.pool.get().await?;
        let today = clock.now().format("%Y-%m-%d").to_string();
        let row: Option<(i32, i32, bool)> = schema::daily_usage::table
            .filter(schema::daily_usage::date.eq(&today))
            .filter(schema::daily_usage::user_id.eq(uid.0 as i32))
            .filter(schema::daily_usage::app_id.eq(app_id.as_ref()))
            .select((
                schema::daily_usage::closed_millis,
                schema::daily_usage::open_millis,
                schema::daily_usage::extended,
            ))
            .first(&mut conn)
            .await
            .ok();

        let (closed, open, extended) = row.unwrap_or((0, 0, false));
        Ok((closed as i64 + open as i64, extended))
    }

    // ── event writes ───────────────────────────────────────────────

    pub async fn write_event(
        &self,
        event_type: i32,
        user_id: i32,
        timestamp: i64,
        app_id: Option<&str>,
        title: Option<&str>,
    ) -> anyhow::Result<()> {
        let mut conn = self.pool.get().await?;
        diesel::insert_into(schema::events::table)
            .values((
                schema::events::event_type.eq(event_type),
                schema::events::user_id.eq(user_id),
                schema::events::timestamp.eq(timestamp),
                schema::events::app_id.eq(app_id),
                schema::events::title.eq(title),
            ))
            .execute(&mut conn)
            .await?;
        Ok(())
    }

    /// Batch INSERT buffered events into the events table in a single transaction.
    /// Daily usage is NOT accumulated here; it is materialized by
    /// `apply_closed_deltas_from_buffer` in the same transaction.
    pub async fn flush_events<Conn>(
        &self,
        conn: &mut Conn,
        events: &[BufferedEvent],
    ) -> anyhow::Result<()>
    where
        Conn: AsyncConnection<Backend = diesel::sqlite::Sqlite>,
    {
        for event in events {
            let app_id = if event.event_type == EVENT_WINDOW_FOCUSED
                || event.event_type == EVENT_IDLE
                || event.event_type == EVENT_RESUMED
            {
                Some(event.app_id.as_ref())
            } else {
                None
            };
            let title = event.title.as_ref().map(|t| t.as_str());
            diesel::insert_into(schema::events::table)
                .values((
                    schema::events::event_type.eq(event.event_type),
                    schema::events::user_id.eq(event.uid.0 as i32),
                    schema::events::timestamp.eq(event.timestamp.timestamp_millis()),
                    schema::events::app_id.eq(app_id),
                    schema::events::title.eq(title),
                ))
                .execute(conn)
                .await?;
        }
        Ok(())
    }

    // ── daily usage materialization ─────────────────────────────────

    /// Set the `extended` flag on today's daily_usage row (upsert).
    pub async fn mark_daily_usage_extended(
        &self,
        app_id: &AppId,
        today: &str,
        uid: Uid,
    ) -> anyhow::Result<()> {
        let mut conn = self.pool.get().await?;
        let affected = diesel::update(
            schema::daily_usage::table
                .filter(schema::daily_usage::date.eq(today))
                .filter(schema::daily_usage::user_id.eq(uid.0 as i32))
                .filter(schema::daily_usage::app_id.eq(app_id.as_ref())),
        )
        .set((schema::daily_usage::extended.eq(true),))
        .execute(&mut conn)
        .await?;

        if affected == 0 {
            diesel::insert_into(schema::daily_usage::table)
                .values((
                    schema::daily_usage::date.eq(today),
                    schema::daily_usage::user_id.eq(uid.0 as i32),
                    schema::daily_usage::app_id.eq(app_id.as_ref()),
                    schema::daily_usage::closed_millis.eq(0),
                    schema::daily_usage::open_millis.eq(0),
                    schema::daily_usage::extended.eq(true),
                ))
                .execute(&mut conn)
                .await?;
        }
        Ok(())
    }

    /// Apply closed-interval deltas from a buffered event batch.
    ///
    /// This is called after `flush_events` in the same transaction.
    /// It walks only the buffered events, pairing WindowFocused with
    /// close events to compute durations. Pre-buffer intervals are
    /// resolved with a single-row lookup in the events table.
    ///
    /// Open intervals (WindowFocused not followed by a close event in
    /// the buffer) get a row with `open_millis = 0`; actual open time
    /// is accumulated by `increment_open_ms` on each minute-tick.
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

        let mut open_focus: std::collections::HashMap<Uid, &BufferedEvent> =
            std::collections::HashMap::new();

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
                    )
                    .await?;
                } else {
                    Self::apply_pre_buffer_close(conn, uid, event, today_start_millis, now_millis)
                        .await?;
                }
            } else if event.event_type == EVENT_WINDOW_FOCUSED {
                open_focus.insert(event.uid, event);
            }
        }

        for (uid, focus) in open_focus {
            let date = focus.timestamp.format("%Y-%m-%d").to_string();
            Self::ensure_row_for_open(conn, &date, uid, &focus.app_id).await?;
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

        // Read the last WindowFocused event timestamp for this uid+app today.
        let open_ms = match schema::events::table
            .filter(schema::events::user_id.eq(uid.0 as i32))
            .filter(schema::events::event_type.eq(EVENT_WINDOW_FOCUSED))
            // Filter by app_id so we don't attribute another app's focus start
            // time to this app when the last WindowFocused happens to be for a
            // different app (which would produce a negative or wrong duration).
            .filter(schema::events::app_id.eq(app_id.as_ref()))
            .filter(schema::events::timestamp.ge(today_start_millis))
            .filter(schema::events::timestamp.le(now_millis))
            .order(schema::events::timestamp.desc())
            .limit(1)
            .select(schema::events::timestamp)
            .first::<i64>(conn)
            .await
        {
            Ok(ts) => Self::duration_millis(ts, now_millis),
            Err(diesel::result::Error::NotFound) => 0,
            Err(e) => return Err(e.into()),
        };

        diesel::insert_into(schema::daily_usage::table)
            .values((
                schema::daily_usage::date.eq(&today),
                schema::daily_usage::user_id.eq(uid.0 as i32),
                schema::daily_usage::app_id.eq(app_id.as_ref()),
                schema::daily_usage::closed_millis.eq(0),
                schema::daily_usage::open_millis.eq(open_ms as i32),
                schema::daily_usage::extended.eq(false),
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

        Ok(())
    }

    // ── helpers ────────────────────────────────────────────────────

    /// Upsert a closed-interval delta: add to closed_millis, zero open_millis.
    ///
    /// Uses `ON CONFLICT DO UPDATE` to atomically increment `closed_millis`
    /// without a prior SELECT — the UPSERT handles both insert and update
    /// in a single round-trip.
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
                schema::daily_usage::extended.eq(false),
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
                schema::daily_usage::extended.eq(false),
            ))
            .execute(conn)
            .await
    }

    /// Return the raw millisecond difference between two epoch-millis timestamps.
    ///
    /// No rounding — caller accumulates milliseconds and converts to
    /// minutes only at policy/display boundaries.
    pub(crate) fn duration_millis(start: i64, end: i64) -> i64 {
        (end - start).max(0)
    }

    /// Upsert a closed interval into daily_usage, splitting across UTC-day
    /// boundaries when the interval spans midnight.
    async fn upsert_closed_delta_for_pair<Conn>(
        conn: &mut Conn,
        uid: Uid,
        app_id: &AppId,
        focus_ts: &DateTime<Utc>,
        close_ts: &DateTime<Utc>,
    ) -> anyhow::Result<()>
    where
        Conn: AsyncConnection<Backend = diesel::sqlite::Sqlite>,
    {
        Self::upsert_interval_split_days(conn, uid, app_id, focus_ts, close_ts).await
    }

    /// Split a focus interval across UTC-day boundaries and upsert each day
    /// segment into daily_usage.
    ///
    /// A focus interval starting at 23:30 and ending at 00:30 the next day
    /// produces two rows: 30 min attributed to the start day, 30 min to the end
    /// day.
    async fn upsert_interval_split_days<Conn>(
        conn: &mut Conn,
        uid: Uid,
        app_id: &AppId,
        focus_ts: &DateTime<Utc>,
        close_ts: &DateTime<Utc>,
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
                break;
            }

            let dur = Self::duration_millis(
                seg_start.timestamp_millis(),
                next_boundary.timestamp_millis(),
            );
            let date = seg_start.format("%Y-%m-%d").to_string();
            Self::upsert_closed_delta(conn, &date, uid, app_id, dur).await?;
            seg_start = next_boundary;
        }

        Ok(())
    }

    /// Query the last WindowFocused event before the buffer for a uid,
    /// returning `(timestamp_millis, app_id)` in a single round-trip.
    async fn resolve_pre_buffer_focus<Conn>(
        conn: &mut Conn,
        uid: Uid,
        today_start_millis: i64,
        now_millis: i64,
    ) -> Option<(i64, Option<String>)>
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
            .select((schema::events::timestamp, schema::events::app_id))
            .first::<(i64, Option<String>)>(conn)
            .await
            .ok()
    }

    /// Handle a close event whose interval started before the buffer:
    /// query the last focus event, resolve the app_id, and upsert.
    ///
    /// app_id is always resolved from the events table (the last
    /// WindowFocused for this uid). The close event's own `app_id` is
    /// ignored — Unfocused events no longer carry one.
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
        let Some((start_ts, db_app_id)) =
            Self::resolve_pre_buffer_focus(conn, uid, today_start_millis, now_millis).await
        else {
            return Ok(());
        };
        let Some(ref resolved) = db_app_id else {
            return Ok(());
        };
        let valid = AppId::new(resolved).unwrap_or_else(|_| AppId::new("unknown").unwrap());

        let start_dt = DateTime::from_timestamp_millis(start_ts)
            .ok_or_else(|| anyhow::anyhow!("resolved focus timestamp must be valid: {start_ts}"))?;
        Self::upsert_interval_split_days(conn, uid, &valid, &start_dt, &event.timestamp).await?;

        Ok(())
    }
}

#[derive(Debug, Clone, diesel::Queryable, diesel::Selectable)]
#[diesel(table_name = crate::store::schema::events)]
pub struct EventRow {
    pub id: i32,
    pub event_type: i32,
    pub user_id: i32,
    pub timestamp: i64,
    pub app_id: Option<String>,
    pub title: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blocking::EnforcerActor;
    use crate::blocking::test_support::{MockPlatform, app, dt, setup};
    use crate::platform::PlatformEvent;
    use crate::signal::DaemonSignal;
    use crate::store::StoreBuilder;

    use diesel::{ExpressionMethods, QueryDsl};
    use diesel_async::RunQueryDsl;
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::time::SystemTime;
    use tokio::sync::RwLock;
    use tokio::sync::mpsc;
    use wellbeing_core::{ActiveBlockEntry, Pid, SystemClock, VirtualClock, WindowTitle};

    // ── flush emits signal ──

    #[tokio::test]
    async fn test_flush_emits_signal() {
        let (_tmp, _pool, mut actor, mut signal_rx) = setup().await;

        actor.event_buffer.push(BufferedEvent {
            uid: Uid(1000),
            app_id: app("firefox"),
            event_type: 0, // EVENT_WINDOW_FOCUSED
            timestamp: dt(1_000_000),
            title: None,
        });
        actor.event_buffer.push(BufferedEvent {
            uid: Uid(1000),
            app_id: app("firefox"),
            event_type: 1, // EVENT_UNFOCUSED
            timestamp: dt(1_000_100),
            title: None,
        });

        actor.flush_buffer().await.expect("flush should succeed");

        // DailyUsageChanged emitted for uid in buffer
        match signal_rx.try_recv() {
            Ok(DaemonSignal::DailyUsageChanged { uid }) => assert_eq!(uid, 1000),
            other => panic!("Expected DailyUsageChanged, got {:?}", other),
        }
        assert!(signal_rx.try_recv().is_err());
    }

    // ── empty flush produces no signal ──

    #[tokio::test]
    async fn test_empty_flush_no_signal() {
        let (_tmp, _pool, mut actor, mut signal_rx) = setup().await;

        let result = actor.flush_buffer().await;

        assert!(result.is_ok(), "empty flush should succeed");
        assert!(signal_rx.try_recv().is_err(), "no signal on empty flush");
    }

    // ── count trigger flushes at 100 events ──

    #[tokio::test]
    async fn test_count_trigger_flushes_at_100_events() {
        let (_tmp, _pool, mut actor, _signal_rx) = setup().await;

        for i in 0..99 {
            let app_id = AppId::new(&format!("test.app.{i}")).unwrap();
            actor
                .handle_event(PlatformEvent::WindowFocused {
                    app_id,
                    title: WindowTitle::new("test"),
                    pid: Pid(1234),
                    uid: Uid(i + 1),
                    overlay_shown: false,
                })
                .await;
        }
        assert_eq!(actor.event_buffer.len(), 99);

        actor
            .handle_event(PlatformEvent::WindowFocused {
                app_id: AppId::new("test.app.100").unwrap(),
                title: WindowTitle::new("test"),
                pid: Pid(1234),
                uid: Uid(1000),
                overlay_shown: false,
            })
            .await;

        assert!(actor.event_buffer.is_empty());
    }

    // ── recover emits daily usage changed ──

    #[tokio::test]
    async fn test_recover_emits_daily_usage_changed() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let pool = StoreBuilder::new(tmp.path().to_path_buf())
            .build()
            .await
            .unwrap();
        let platform = Arc::new(MockPlatform);
        let (signal_tx, mut signal_rx) = mpsc::unbounded_channel();
        let active_blocks = Arc::new(RwLock::new(HashMap::new()));
        let (mut enforcer, _) = EnforcerActor::new(
            pool.clone(),
            platform,
            SystemClock,
            signal_tx,
            active_blocks,
        );

        let uid = Uid(1000);
        let now = SystemClock.now();
        let today_date = now.format("%Y-%m-%d").to_string();
        let ts_millis = chrono::NaiveDateTime::parse_from_str(
            &format!("{} 10:00:00", today_date),
            "%Y-%m-%d %H:%M:%S",
        )
        .unwrap()
        .and_utc()
        .timestamp_millis();
        enforcer
            .repo
            .write_event(0, uid.0 as i32, ts_millis, Some("firefox"), None)
            .await
            .unwrap();

        enforcer.recover().await.unwrap();

        match signal_rx.try_recv() {
            Ok(DaemonSignal::DailyUsageChanged { uid: u }) => assert_eq!(u, 1000),
            other => panic!("Expected DailyUsageChanged, got {:?}", other),
        }
    }

    // ── grant extension writes immediately ──

    #[tokio::test]
    async fn test_grant_extension_writes_immediately() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let pool = StoreBuilder::new(tmp.path().to_path_buf())
            .build()
            .await
            .unwrap();
        let platform = Arc::new(MockPlatform);
        let (signal_tx, _) = mpsc::unbounded_channel();
        let uid = Uid(1000);
        let app_id = AppId::new("test.app").unwrap();
        let active_blocks = Arc::new(RwLock::new(
            std::iter::once((
                uid,
                std::iter::once((
                    app_id.clone(),
                    ActiveBlockEntry {
                        app_id: app_id.as_ref().to_string(),
                        policy_id: 42,
                        reason: 0,
                        blocked_since: SystemTime::now()
                            .duration_since(SystemTime::UNIX_EPOCH)
                            .unwrap()
                            .as_millis() as u64,
                        available_actions: vec![0],
                    },
                ))
                .collect::<HashMap<_, _>>(),
            ))
            .collect::<HashMap<_, _>>(),
        ));
        let (mut enforcer, _) = EnforcerActor::new(
            pool.clone(),
            platform,
            SystemClock,
            signal_tx,
            active_blocks,
        );

        enforcer.current_focus.insert(uid, app_id.clone());

        enforcer.handle_user_action(app_id.clone(), 0, uid).await;

        let mut conn = pool.get().await.unwrap();

        let event_count: i64 = crate::store::schema::events::table
            .count()
            .get_result(&mut conn)
            .await
            .unwrap();
        assert_eq!(event_count, 1);

        let today = enforcer.clock.now().format("%Y-%m-%d").to_string();
        let extended_flag: bool = crate::store::schema::daily_usage::table
            .select(crate::store::schema::daily_usage::extended)
            .filter(crate::store::schema::daily_usage::date.eq(&today))
            .filter(crate::store::schema::daily_usage::user_id.eq(uid.0 as i32))
            .filter(crate::store::schema::daily_usage::app_id.eq(app_id.as_ref()))
            .first(&mut conn)
            .await
            .unwrap();
        assert!(extended_flag);

        assert!(enforcer.event_buffer.is_empty());
    }

    // ── daily usage equivalence ──

    #[tokio::test]
    async fn test_daily_usage_equivalence() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let pool = StoreBuilder::new(tmp.path().to_path_buf())
            .build()
            .await
            .unwrap();
        let platform = Arc::new(MockPlatform);
        let (signal_tx, _) = mpsc::unbounded_channel();
        let active_blocks = Arc::new(RwLock::new(HashMap::new()));

        let start = Utc::now();
        let inner_clock = Arc::new(std::sync::Mutex::new(VirtualClock::new(start)));
        let test_clock = inner_clock.clone();

        struct SharedClock(Arc<std::sync::Mutex<VirtualClock>>);

        impl Clock for SharedClock {
            fn now(&self) -> DateTime<Utc> {
                self.0.lock().unwrap().now()
            }
        }

        let shared = SharedClock(inner_clock);
        let (mut enforcer, _) =
            EnforcerActor::new(pool.clone(), platform, shared, signal_tx, active_blocks);

        let uid = Uid(1000);

        enforcer
            .handle_event(PlatformEvent::WindowFocused {
                app_id: AppId::new("firefox").unwrap(),
                title: WindowTitle::new("Mozilla Firefox"),
                pid: Pid(1234),
                uid,
                overlay_shown: false,
            })
            .await;

        test_clock
            .lock()
            .unwrap()
            .advance(chrono::Duration::minutes(10));

        enforcer
            .handle_event(PlatformEvent::WindowFocused {
                app_id: AppId::new("code").unwrap(),
                title: WindowTitle::new("Visual Studio Code"),
                pid: Pid(5678),
                uid,
                overlay_shown: false,
            })
            .await;

        test_clock
            .lock()
            .unwrap()
            .advance(chrono::Duration::minutes(5));

        enforcer.handle_event(PlatformEvent::Unfocused).await;
        enforcer.flush_buffer().await.unwrap();

        let mut conn = pool.get().await.unwrap();

        let rows: Vec<(String, i32, String, i32, i32, bool)> =
            crate::store::schema::daily_usage::table
                .load(&mut conn)
                .await
                .unwrap();

        assert_eq!(rows.len(), 2);

        let today = start.format("%Y-%m-%d").to_string();
        for (date, user_id, app_id, closed_millis, open_millis, _extended) in &rows {
            assert_eq!(date, &today);
            assert_eq!(*user_id, 1000);
            assert_eq!(*open_millis, 0); // both intervals are closed
            match app_id.as_str() {
                "firefox" => assert_eq!(*closed_millis, 600_000),
                "code" => assert_eq!(*closed_millis, 300_000),
                other => panic!("unexpected app_id: {other}"),
            }
        }
    }

    // ── idempotent focus switches ──

    #[tokio::test]
    async fn test_same_app_focus_does_not_buffer_unfocused() {
        let (_tmp, _pool, mut actor, _signal_rx) = setup().await;

        actor
            .handle_event(PlatformEvent::WindowFocused {
                app_id: app("firefox"),
                title: WindowTitle::new("test"),
                pid: Pid(1234),
                uid: Uid(1000),
                overlay_shown: false,
            })
            .await;

        actor
            .handle_event(PlatformEvent::WindowFocused {
                app_id: app("firefox"),
                title: WindowTitle::new("test"),
                pid: Pid(1234),
                uid: Uid(1000),
                overlay_shown: false,
            })
            .await;

        assert_eq!(actor.event_buffer.len(), 1);
        assert_eq!(actor.current_focus.get(&Uid(1000)), Some(&app("firefox")));
    }

    // ── recover handles no open intervals ──

    #[tokio::test]
    async fn test_recover_no_open_intervals() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let pool = StoreBuilder::new(tmp.path().to_path_buf())
            .build()
            .await
            .unwrap();
        let platform = Arc::new(MockPlatform);
        let (signal_tx, _) = mpsc::unbounded_channel();
        let active_blocks = Arc::new(RwLock::new(HashMap::new()));
        let (mut enforcer, _) = EnforcerActor::new(
            pool.clone(),
            platform,
            SystemClock,
            signal_tx,
            active_blocks,
        );

        let uid = Uid(1000);
        let ts_millis =
            chrono::NaiveDateTime::parse_from_str("2025-01-01 10:00:00", "%Y-%m-%d %H:%M:%S")
                .unwrap()
                .and_utc()
                .timestamp_millis();
        enforcer
            .repo
            .write_event(1, uid.0 as i32, ts_millis, Some("firefox"), None)
            .await
            .unwrap();

        enforcer.recover().await.unwrap();

        assert!(enforcer.current_focus.is_empty());
    }

    // ── cross-batch close event pairs with pre-batch open ──

    #[tokio::test]
    async fn test_cross_batch_close_does_not_lose_time() {
        // Regression test: when a close event arrives in a DIFFERENT batch
        // than its matching WindowFocused, and the new batch also contains a
        // WindowFocused for a different app, apply_closed_deltas_from_buffer's
        // `else` branch queries the events table for the last WindowFocused.
        //
        // If flush_events runs before apply_closed_deltas_from_buffer, the
        // query finds the NEW WindowFocused (different app) from the same
        // batch — producing 0ms duration and silently losing the tracked time.
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let pool = StoreBuilder::new(tmp.path().to_path_buf())
            .build()
            .await
            .unwrap();
        let platform = Arc::new(MockPlatform);
        let (signal_tx, _) = mpsc::unbounded_channel();
        let active_blocks = Arc::new(RwLock::new(HashMap::new()));

        let start = Utc::now();
        let inner = Arc::new(std::sync::Mutex::new(VirtualClock::new(start)));
        let test_clock = inner.clone();
        struct SharedClock(Arc<std::sync::Mutex<VirtualClock>>);
        impl Clock for SharedClock {
            fn now(&self) -> DateTime<Utc> {
                self.0.lock().unwrap().now()
            }
        }

        let shared = SharedClock(inner);
        let (mut enforcer, _) =
            EnforcerActor::new(pool.clone(), platform, shared, signal_tx, active_blocks);
        let uid = Uid(1000);

        // ── Batch 1 ── focus Alacritty, then flush
        enforcer
            .handle_event(PlatformEvent::WindowFocused {
                app_id: AppId::new("Alacritty").unwrap(),
                title: WindowTitle::new("alacritty"),
                pid: Pid(1),
                uid,
                overlay_shown: false,
            })
            .await;
        enforcer.flush_buffer().await.unwrap();

        // Advance clock by 5 minutes, then switch to brave-browser.
        // This buffers Unf(Alacritty) + WF(brave) in batch 2.
        test_clock
            .lock()
            .unwrap()
            .advance(chrono::Duration::minutes(5));

        enforcer
            .handle_event(PlatformEvent::WindowFocused {
                app_id: AppId::new("brave-browser").unwrap(),
                title: WindowTitle::new("Brave"),
                pid: Pid(2),
                uid,
                overlay_shown: false,
            })
            .await;

        // ── Batch 2 ── flush the switch
        enforcer.flush_buffer().await.unwrap();

        // ── Verify ──
        let mut conn = pool.get().await.unwrap();
        let rows: Vec<(String, i32, String, i32, i32, bool)> =
            crate::store::schema::daily_usage::table
                .load(&mut conn)
                .await
                .unwrap();

        // Should have 2 rows: Alacritty with 5 min closed, brave with 0 open
        assert_eq!(rows.len(), 2, "expected 2 apps in daily_usage");

        let today = start.format("%Y-%m-%d").to_string();
        for (date, user_id, app_id, closed_millis, open_millis, _extended) in &rows {
            assert_eq!(date, &today);
            assert_eq!(*user_id, 1000);
            match app_id.as_str() {
                "Alacritty" => {
                    assert!(
                        *closed_millis >= 300_000,
                        "Alacritty closed_millis={closed_millis} should be ≥300000"
                    );
                    assert_eq!(*open_millis, 0, "Alacritty open_millis should be 0");
                }
                "brave-browser" => {
                    assert_eq!(*closed_millis, 0, "brave closed_millis should be 0");
                }
                other => panic!("unexpected app: {other}"),
            }
        }
    }
}
