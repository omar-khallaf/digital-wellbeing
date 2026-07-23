//! Persistence layer for the blocking/enforcement feature.
//!
//! [`BlockingRepo`] owns all database access. Domain logic (policy
//! evaluation, schedule filtering) is NOT in this module — it stays
//! in `core.rs` / `policy::core`.

use chrono::{DateTime, Utc};
use diesel::QueryResult;
use diesel::{ExpressionMethods, QueryDsl};
use diesel_async::{AsyncConnection, RunQueryDsl};
use serde_json::json;
use wellbeing_core::{AppId, CategoryId, Clock, PolicyId, Uid};

use super::super::buffer::BufferedEvent;

use crate::policy::{DieselPolicyRepo, Policy, PolicyConfig, PolicyRepo as _};
use crate::store::{DbPool, schema};

pub const EVENT_WINDOW_FOCUSED: i32 = 0;
pub const EVENT_UNFOCUSED: i32 = 1;
pub const EVENT_IDLE: i32 = 2;
pub const EVENT_RESUMED: i32 = 3;
pub const EVENT_SLEPT: i32 = 4;
pub const EVENT_SHUT_DOWN: i32 = 5;
pub const EVENT_LOCKED: i32 = 6;
pub const EVENT_LOGGED_OUT: i32 = 7;

pub const CLOSE_EVENT_TYPES: &[i32] = &[
    EVENT_UNFOCUSED,
    EVENT_SLEPT,
    EVENT_SHUT_DOWN,
    EVENT_LOCKED,
    EVENT_LOGGED_OUT,
];

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

    /// Fetch a single policy by id, returning `PolicyConfig` or `None`.
    #[allow(dead_code)]
    pub async fn fetch_policy_config(
        &self,
        policy_id: PolicyId,
    ) -> anyhow::Result<Option<PolicyConfig>> {
        let mut conn = self.pool.get().await?;
        let row = DieselPolicyRepo
            .get_policy(&mut conn, policy_id.0 as i32)
            .await?;
        Ok(row.map(|r| PolicyConfig::from(r.into_domain_policy())))
    }

    // ── event writes ───────────────────────────────────────────────

    pub async fn write_event(
        &self,
        event_type: i32,
        payload: &str,
        user_id: i32,
    ) -> anyhow::Result<()> {
        let mut conn = self.pool.get().await?;
        diesel::insert_into(schema::events::table)
            .values((
                schema::events::event_type.eq(event_type),
                schema::events::payload.eq(payload),
                schema::events::user_id.eq(user_id),
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
            let now_str = event.timestamp.format("%Y-%m-%d %H:%M:%S").to_string();
            let payload = match event.event_type {
                EVENT_WINDOW_FOCUSED => {
                    json!({"t": &now_str, "a": event.app_id.as_ref()}).to_string()
                }
                _ => json!({"t": &now_str}).to_string(),
            };
            diesel::insert_into(schema::events::table)
                .values((
                    schema::events::event_type.eq(event.event_type),
                    schema::events::payload.eq(&payload),
                    schema::events::user_id.eq(event.uid.0 as i32),
                ))
                .execute(&mut *conn)
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

        let today = now.format("%Y-%m-%d").to_string();
        let now_str = now.format("%Y-%m-%d %H:%M:%S").to_string();

        let mut open_focus: std::collections::HashMap<Uid, &BufferedEvent> =
            std::collections::HashMap::new();

        for event in events {
            if CLOSE_EVENT_TYPES.contains(&event.event_type) {
                let uid = event.uid;
                if let Some(focus) = open_focus.remove(&uid) {
                    Self::upsert_closed_delta_for_pair(
                        conn,
                        &today,
                        uid,
                        &focus.app_id,
                        &focus.timestamp,
                        &event.timestamp,
                    )
                    .await?;
                } else {
                    Self::apply_pre_buffer_close(conn, &today, &now_str, uid, event).await?;
                }
            } else if event.event_type == EVENT_WINDOW_FOCUSED {
                open_focus.insert(event.uid, event);
            }
        }

        for (uid, focus) in open_focus {
            Self::ensure_row_for_open(conn, &today, uid, &focus.app_id).await?;
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
        let now_str = now.format("%Y-%m-%d %H:%M:%S").to_string();

        // Read the last WindowFocused event timestamp for this uid+app today.
        let open_ms = match schema::events::table
            .filter(schema::events::user_id.eq(uid.0 as i32))
            .filter(schema::events::event_type.eq(EVENT_WINDOW_FOCUSED))
            // Filter by app_id so we don't attribute another app's focus start
            // time to this app when the last WindowFocused happens to be for a
            // different app (which would produce a negative or wrong duration).
            .filter(schema::events::app_id.eq(app_id.as_ref()))
            .filter(schema::events::timestamp.ge(&format!("{} 00:00:00", today)))
            .filter(schema::events::timestamp.le(&now_str))
            .order(schema::events::id.desc())
            .limit(1)
            .select(schema::events::timestamp)
            .first::<String>(conn)
            .await
        {
            Ok(ts) => Self::duration_millis(&ts, &now_str),
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
        let existing: Option<(i32, bool)> = schema::daily_usage::table
            .filter(schema::daily_usage::date.eq(today))
            .filter(schema::daily_usage::user_id.eq(uid.0 as i32))
            .filter(schema::daily_usage::app_id.eq(app_id.as_ref()))
            .select((
                schema::daily_usage::closed_millis,
                schema::daily_usage::extended,
            ))
            .first(conn)
            .await
            .ok();

        let current_closed = existing.map(|(c, _)| c).unwrap_or(0);
        let extended = existing.map(|(_, e)| e).unwrap_or(false);

        diesel::insert_into(schema::daily_usage::table)
            .values((
                schema::daily_usage::date.eq(today),
                schema::daily_usage::user_id.eq(uid.0 as i32),
                schema::daily_usage::app_id.eq(app_id.as_ref()),
                schema::daily_usage::closed_millis.eq((current_closed as i64 + delta_ms) as i32),
                schema::daily_usage::open_millis.eq(0),
                schema::daily_usage::extended.eq(extended),
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

    /// Return the raw millisecond difference between two
    /// `YYYY-MM-DD HH:MM:SS` UTC timestamps.
    ///
    /// No rounding — caller accumulates milliseconds and converts to
    /// minutes only at policy/display boundaries.
    pub(crate) fn duration_millis(start: &str, end: &str) -> i64 {
        let fmt = "%Y-%m-%d %H:%M:%S";
        let parse = |ts: &str| {
            DateTime::parse_from_rfc3339(&format!("{}Z", ts))
                .or_else(|_| DateTime::parse_from_str(ts, fmt))
        };
        let Ok(s) = parse(start) else {
            tracing::warn!("Failed to parse start timestamp: {start}");
            return 0;
        };
        let Ok(e) = parse(end) else {
            tracing::warn!("Failed to parse end timestamp: {end}");
            return 0;
        };
        let diff_secs = e.signed_duration_since(s).num_seconds();
        diff_secs.max(0) * 1000
    }

    async fn upsert_closed_delta_for_pair<Conn>(
        conn: &mut Conn,
        today: &str,
        uid: Uid,
        app_id: &AppId,
        focus_ts: &DateTime<Utc>,
        close_ts: &DateTime<Utc>,
    ) -> anyhow::Result<()>
    where
        Conn: AsyncConnection<Backend = diesel::sqlite::Sqlite>,
    {
        let dur = Self::duration_millis(
            &focus_ts.format("%Y-%m-%d %H:%M:%S").to_string(),
            &close_ts.format("%Y-%m-%d %H:%M:%S").to_string(),
        );
        Self::upsert_closed_delta(conn, today, uid, app_id, dur).await?;
        Ok(())
    }

    /// Query the last WindowFocused event before the buffer for a uid,
    /// returning `(timestamp, app_id)` in a single round-trip.
    async fn resolve_pre_buffer_focus<Conn>(
        conn: &mut Conn,
        uid: Uid,
        today: &str,
        now_str: &str,
    ) -> Option<(String, Option<String>)>
    where
        Conn: AsyncConnection<Backend = diesel::sqlite::Sqlite>,
    {
        schema::events::table
            .filter(schema::events::user_id.eq(uid.0 as i32))
            .filter(schema::events::event_type.eq(EVENT_WINDOW_FOCUSED))
            .filter(schema::events::timestamp.ge(&format!("{} 00:00:00", today)))
            .filter(schema::events::timestamp.le(&now_str))
            .order(schema::events::id.desc())
            .limit(1)
            .select((schema::events::timestamp, schema::events::app_id))
            .first::<(String, Option<String>)>(conn)
            .await
            .ok()
    }

    /// Handle a close event whose interval started before the buffer:
    /// query the last focus event, resolve the app_id, and upsert.
    async fn apply_pre_buffer_close<Conn>(
        conn: &mut Conn,
        today: &str,
        now_str: &str,
        uid: Uid,
        event: &BufferedEvent,
    ) -> anyhow::Result<()>
    where
        Conn: AsyncConnection<Backend = diesel::sqlite::Sqlite>,
    {
        let Some((start_ts, db_app_id)) =
            Self::resolve_pre_buffer_focus(conn, uid, today, now_str).await
        else {
            return Ok(());
        };

        let dur = Self::duration_millis(
            &start_ts,
            &event.timestamp.format("%Y-%m-%d %H:%M:%S").to_string(),
        );

        let app_id_str = event.app_id.as_ref();
        let resolved = if !app_id_str.is_empty() {
            app_id_str
        } else if let Some(ref db_id) = db_app_id {
            db_id.as_str()
        } else {
            return Ok(());
        };
        let valid = AppId::new(resolved).unwrap_or_else(|_| AppId::new("unknown").unwrap());
        Self::upsert_closed_delta(conn, today, uid, &valid, dur).await?;

        Ok(())
    }
}

#[derive(Debug, Clone, diesel::Queryable, diesel::Selectable)]
#[diesel(table_name = crate::store::schema::events)]
pub struct EventRow {
    pub event_type: i32,
    pub user_id: i32,
    pub timestamp: String,
    pub app_id: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blocking::EnforcerActor;
    use crate::platform::{Platform, PlatformEvent};
    use crate::signal::DaemonSignal;
    use crate::store::StoreBuilder;

    use chrono::TimeZone;
    use diesel::{ExpressionMethods, QueryDsl};
    use diesel_async::RunQueryDsl;
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::time::SystemTime;
    use tempfile::TempDir;
    use tokio::sync::RwLock;
    use tokio::sync::mpsc;
    use wellbeing_core::{ActiveBlockEntry, Pid, SystemClock, VirtualClock, WindowTitle};

    // ── mock platform ──

    struct MockPlatform;

    impl Platform for MockPlatform {
        type EventStream = tokio_stream::wrappers::UnboundedReceiverStream<PlatformEvent>;

        async fn notify(&self, _title: &str, _body: &str) -> anyhow::Result<()> {
            Ok(())
        }
    }

    // ── helpers ──

    fn app(s: &str) -> AppId {
        AppId::new(s).unwrap()
    }

    fn dt(secs: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(secs, 0).unwrap()
    }

    /// Create an EnforcerActor with a real SQLite database and mocks.
    async fn setup() -> (
        TempDir,
        DbPool,
        EnforcerActor<MockPlatform, VirtualClock>,
        mpsc::UnboundedReceiver<DaemonSignal>,
    ) {
        let tmp = TempDir::new().expect("temp dir");
        let db_path = tmp.path().join("test.db");
        let pool = StoreBuilder::new(db_path)
            .build()
            .await
            .expect("build store");
        let (signal_tx, signal_rx) = mpsc::unbounded_channel();
        let platform = Arc::new(MockPlatform);
        let clock = VirtualClock::new(dt(1_000_000));
        let active_blocks = Arc::new(RwLock::new(
            HashMap::<Uid, HashMap<AppId, ActiveBlockEntry>>::new(),
        ));

        let (actor, _) =
            EnforcerActor::new(pool.clone(), platform, clock, signal_tx, active_blocks);

        (tmp, pool, actor, signal_rx)
    }

    // ── flush emits signal ──

    #[tokio::test]
    async fn test_flush_emits_signal() {
        let (_tmp, _pool, mut actor, mut signal_rx) = setup().await;

        actor.event_buffer.push(BufferedEvent {
            uid: Uid(1000),
            app_id: app("firefox"),
            event_type: 0, // EVENT_WINDOW_FOCUSED
            timestamp: dt(1_000_000),
        });
        actor.event_buffer.push(BufferedEvent {
            uid: Uid(1000),
            app_id: app("firefox"),
            event_type: 1, // EVENT_UNFOCUSED
            timestamp: dt(1_000_100),
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
        let today = SystemClock.now().format("%Y-%m-%d").to_string();
        let ts = format!("{} 10:00:00", today);
        let payload = json!({"t": ts, "a": "firefox"}).to_string();
        enforcer
            .repo
            .write_event(0, &payload, uid.0 as i32)
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
        let payload = json!({"t": "2025-01-01 10:00:00"}).to_string();
        enforcer
            .repo
            .write_event(1, &payload, uid.0 as i32)
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
