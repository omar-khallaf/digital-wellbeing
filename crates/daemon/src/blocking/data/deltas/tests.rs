use chrono::{DateTime, Utc};
use diesel::ExpressionMethods;
use diesel_async::RunQueryDsl;

use tempfile::TempDir;

use crate::platform::PlatformEvent;
use wellbeing_core::event_types::EVENT_WINDOW_FOCUSED;
use wellbeing_core::{AppId, Uid, WindowTitle};

use crate::store::StoreBuilder;
use crate::store::schema::daily_usage;

use super::BlockingRepo;
use crate::blocking::domain::TimedEvent;

fn app(s: &str) -> AppId {
    AppId::new(s).unwrap()
}

fn dt(ms: i64) -> chrono::DateTime<Utc> {
    DateTime::from_timestamp_millis(ms).unwrap()
}

/// Extract deduplicated UIDs from a slice of timed events.
fn event_uids(events: &[TimedEvent]) -> Vec<Uid> {
    let mut seen = std::collections::HashSet::new();
    events
        .iter()
        .filter(|te| seen.insert(te.event.uid()))
        .map(|te| te.event.uid())
        .collect()
}

fn focus_event(uid: u32, app_name: &str, ts: i64) -> TimedEvent {
    TimedEvent {
        event: PlatformEvent::Focus {
            app_id: app(app_name),
            title: WindowTitle::new("test"),
            pid: wellbeing_core::Pid(1),
            uid: Uid(uid),
        },
        timestamp: dt(ts),
    }
}

fn close_event(uid: u32, app_name: &str, event_type: i32, ts: i64) -> TimedEvent {
    let event = match event_type {
        1 => PlatformEvent::Unfocus { uid: Uid(uid) },
        8 => PlatformEvent::Block {
            app_id: app(app_name),
            title: WindowTitle::new("test"),
            uid: Uid(uid),
        },
        _ => unreachable!(),
    };
    TimedEvent {
        event,
        timestamp: dt(ts),
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
///
/// `app_id` should be `None` for close events (type 1) and system
/// events (types 4‑7) where the DB `CHECK` constraint requires a NULL
/// app_id.
async fn write_raw_event(
    repo: &BlockingRepo,
    event_type: i32,
    uid: u32,
    app_id: Option<&str>,
    ts: i64,
) {
    let mut conn = repo.pool.get().await.unwrap();
    diesel::insert_into(crate::store::schema::events::table)
        .values((
            crate::store::schema::events::event_type.eq(event_type),
            crate::store::schema::events::user_id.eq(uid as i32),
            crate::store::schema::events::timestamp.eq(ts),
            crate::store::schema::events::app_id.eq(app_id.map(|s| s.to_string())),
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
) -> Option<i64> {
    use diesel::QueryDsl;

    let mut conn = repo.pool.get().await.unwrap();
    daily_usage::table
        .filter(daily_usage::date.eq(date))
        .filter(daily_usage::user_id.eq(uid as i32))
        .filter(daily_usage::app_id.eq(app_id))
        .select(daily_usage::closed_millis)
        .first::<i64>(&mut conn)
        .await
        .ok()
}

#[tokio::test]
async fn test_pre_buffer_focus_is_closed_by_close_event() {
    // Given: a pre-buffer WindowFocused at ts=1000 for uid=1000, app=firefox
    let (_tmp, repo) = setup_repo().await;
    write_raw_event(
        &repo,
        EVENT_WINDOW_FOCUSED,
        1000,
        Some("firefox"),
        1_000_000,
    )
    .await;

    // When: a close event arrives in the buffer at ts=3000
    let events = vec![close_event(1000, "firefox", 1, 3_000_000)];
    let mut conn = repo.pool.get().await.unwrap();
    let uids = event_uids(&events);
    repo.apply_closed_deltas_from_buffer(&mut conn, &events, &uids, dt(3_000_000))
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
    let uids = event_uids(&events);
    repo.apply_closed_deltas_from_buffer(&mut conn, &events, &uids, dt(1_000_100))
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
    let uids = event_uids(&events);
    repo.apply_closed_deltas_from_buffer(&mut conn, &events, &uids, dt(1_001_000))
        .await
        .unwrap();

    // Then: firefox interval (1_000_000 → 1_001_000) = 1000ms closed
    let closed = read_closed_millis(&repo, "1970-01-01", 1000, "firefox").await;
    assert_eq!(closed, Some(1000));
}

#[tokio::test]
async fn test_window_focused_closes_pre_buffer_interval_for_different_app() {
    // Given: a pre-buffer WindowFocused at t=1000 for uid=1000, app=firefox
    let (_tmp, repo) = setup_repo().await;
    write_raw_event(
        &repo,
        EVENT_WINDOW_FOCUSED,
        1000,
        Some("firefox"),
        1_000_000,
    )
    .await;

    // When: the next buffer contains a WindowFocused for a DIFFERENT app at t=3000
    let events = vec![focus_event(1000, "chrome", 3_000_000)];
    let mut conn = repo.pool.get().await.unwrap();
    let uids = event_uids(&events);
    repo.apply_closed_deltas_from_buffer(&mut conn, &events, &uids, dt(3_000_000))
        .await
        .unwrap();

    // Then: firefox interval (1_000_000 -> 3_000_000) = 2_000_000ms closed
    let closed = read_closed_millis(&repo, "1970-01-01", 1000, "firefox").await;
    assert_eq!(closed, Some(2_000_000));
}

#[tokio::test]
async fn test_window_focused_closes_pre_buffer_interval_for_same_app() {
    // Given: a pre-buffer WindowFocused at t=1000 for uid=1000, app=firefox
    let (_tmp, repo) = setup_repo().await;
    write_raw_event(
        &repo,
        EVENT_WINDOW_FOCUSED,
        1000,
        Some("firefox"),
        1_000_000,
    )
    .await;

    // When: the next buffer contains a WindowFocused for the SAME app at t=3000
    // (this represents switching back to the same app — still closes prior interval)
    let events = vec![focus_event(1000, "firefox", 3_000_000)];
    let mut conn = repo.pool.get().await.unwrap();
    let uids = event_uids(&events);
    repo.apply_closed_deltas_from_buffer(&mut conn, &events, &uids, dt(3_000_000))
        .await
        .unwrap();

    // Then: firefox interval (1_000_000 -> 3_000_000) = 2_000_000ms closed
    let closed = read_closed_millis(&repo, "1970-01-01", 1000, "firefox").await;
    assert_eq!(closed, Some(2_000_000));
}

// ── Regression tests for pre_buffer ignoring Idle/Resumed ────────────

#[tokio::test]
async fn test_idle_resumed_do_not_break_interval_chaining() {
    // Regression: when Idle(2) / Resumed(3) become the last persisted
    // events between buffer flushes, fetch_last_events_for_uids must
    // look past them and find the preceding Focus so the interval is
    // chained.  Without the INTERVAL_MEASUREMENT_IGNORED_EVENT_TYPES
    // filter the Focus→Idle→Resumed→Focus sequence silently drops the
    // first interval from daily_usage (the ~1 h under-count bug).

    // Given: a pre-buffer Focus at t=1000, then Idle(2) and Resumed(3)
    // at t=2000 / t=2500 (these simulate events flushed between
    // batches that would otherwise break the chain).
    let (_tmp, repo) = setup_repo().await;
    write_raw_event(
        &repo,
        EVENT_WINDOW_FOCUSED,
        1000,
        Some("firefox"),
        1_000_000,
    )
    .await;
    write_raw_event(&repo, 2, 1000, None, 2_000_000).await; // idle
    write_raw_event(&repo, 3, 1000, None, 2_500_000).await; // resumed

    // When: the next buffer delivers a WindowFocused for a different
    // app at t=3000 — this should close the firefox interval.
    let events = vec![focus_event(1000, "chrome", 3_000_000)];
    let mut conn = repo.pool.get().await.unwrap();
    let uids = event_uids(&events);
    repo.apply_closed_deltas_from_buffer(&mut conn, &events, &uids, dt(3_000_000))
        .await
        .unwrap();

    // Then: firefox interval (1_000_000 → 3_000_000) = 2_000_000ms
    // must be recorded as closed.  Without the fix closed_millis is 0
    // or missing (interval silently dropped).
    let closed = read_closed_millis(&repo, "1970-01-01", 1000, "firefox").await;
    assert_eq!(
        closed,
        Some(2_000_000),
        "interval chaining across idle/resumed must preserve closed time"
    );
}

#[tokio::test]
async fn test_close_before_idle_resumed_does_not_chain_interval() {
    // Inverse: when Close(1) is the last non-ignored event (followed
    // only by Idle/Resumed), the interval was already terminated and
    // open_focus must remain None.  A new Focus in the buffer must
    // start a fresh interval, NOT chain back to the pre-Close Focus.

    // Given: Focus(t=1000), Close(t=2000), then Idle(2) / Resumed(3).
    let (_tmp, repo) = setup_repo().await;
    write_raw_event(
        &repo,
        EVENT_WINDOW_FOCUSED,
        1000,
        Some("firefox"),
        1_000_000,
    )
    .await;
    write_raw_event(&repo, 1, 1000, None, 2_000_000).await; // close
    write_raw_event(&repo, 2, 1000, None, 2_500_000).await; // idle
    write_raw_event(&repo, 3, 1000, None, 2_600_000).await; // resumed

    // When: new Focus at t=3000 for a different app.
    let events = vec![focus_event(1000, "chrome", 3_000_000)];
    let mut conn = repo.pool.get().await.unwrap();
    let uids = event_uids(&events);
    repo.apply_closed_deltas_from_buffer(&mut conn, &events, &uids, dt(3_000_000))
        .await
        .unwrap();

    // Then: firefox must NOT have the interval 1_000_000→3_000_000
    // since Close(t=2000) already terminated it.  (The 1_000_000→
    // 2_000_000 portion was closed in a prior flush; the remaining
    // time belongs to the new chrome interval.)
    let closed = read_closed_millis(&repo, "1970-01-01", 1000, "firefox").await;
    assert_eq!(
        closed, None,
        "close event must prevent phantom interval chaining"
    );
}
