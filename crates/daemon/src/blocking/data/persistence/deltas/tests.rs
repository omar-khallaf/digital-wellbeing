use chrono::{DateTime, Utc};
use diesel::ExpressionMethods;
use diesel::QueryDsl;
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
async fn write_raw_event(repo: &BlockingRepo, event_type: i32, uid: u32, app_id: &str, ts: i64) {
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

#[tokio::test]
async fn test_increment_open_ms_does_not_re_add_closed_interval() {
    // Given: a WindowFocused event already in the DB
    let (_tmp, repo) = setup_repo().await;
    write_raw_event(&repo, EVENT_WINDOW_FOCUSED, 1000, "firefox", 1_000_000).await;

    // When: the buffer contains an Unfocused event that closes the interval,
    // and current_focus still contains the uid (simulating the race where
    // increment_open_ms runs after the buffer flush closed the interval)
    let events = vec![close_event(1000, "firefox", 1, 3_000_000)];
    let mut conn = repo.pool.get().await.unwrap();

    // Simulate flush_buffer: apply deltas, flush events, then increment_open_ms
    repo.apply_closed_deltas_from_buffer(&mut conn, &events, dt(3_000_000))
        .await
        .unwrap();
    repo.flush_events(&mut conn, &events).await.unwrap();

    // Simulate increment_open_ms running for the still-focused app
    repo.increment_open_ms(&mut conn, Uid(1000), app("firefox"), dt(3_000_000))
        .await
        .unwrap();

    // Then: closed_millis = 2_000_000, open_millis = 0 (not re-populated)
    let closed = read_closed_millis(&repo, "1970-01-01", 1000, "firefox").await;
    assert_eq!(closed, Some(2_000_000));

    let open: Option<i32> = crate::store::schema::daily_usage::table
        .filter(crate::store::schema::daily_usage::date.eq("1970-01-01"))
        .filter(crate::store::schema::daily_usage::user_id.eq(1000))
        .filter(crate::store::schema::daily_usage::app_id.eq("firefox"))
        .select(crate::store::schema::daily_usage::open_millis)
        .first::<i32>(&mut conn)
        .await
        .ok();
    assert_eq!(
        open,
        Some(0),
        "open_millis should be 0 after interval is closed"
    );
}
