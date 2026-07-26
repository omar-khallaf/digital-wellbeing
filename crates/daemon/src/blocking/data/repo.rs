//! Repository for blocking/enforcement persistence operations.
//!
//! Wraps store DAO calls and provides coarse-grained methods for the
//! [`EnforcerActor`](super::super::core::EnforcerActor).

use chrono::{DateTime, Utc};
use wellbeing_core::{AppClass, Uid};

use crate::blocking::domain::TimedEvent;
use crate::store::DbPool;
use crate::store::daos::apps::AppsDao;
use crate::store::daos::categories::CategoryDao;
use crate::store::daos::daily_usage::deltas;
use crate::store::daos::daily_usage::reads::get_today_app_usage;
use crate::store::daos::events::EventDao;

/// Repository for blocking-feature persistence operations.
pub(crate) struct BlockingRepo {
    pool: DbPool,
}

impl BlockingRepo {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    /// Ensure an app exists in the registry, returning its `apps.id`.
    pub async fn ensure_app(&self, app_class: &AppClass) -> anyhow::Result<i32> {
        let mut conn = self.pool.get().await?;
        AppsDao::ensure_app(&mut conn, app_class).await
    }

    /// Resolve category names for an app (user-specific then system fallback).
    pub async fn resolve_category_names(
        &self,
        app_class: &AppClass,
        uid: Uid,
    ) -> anyhow::Result<Vec<String>> {
        let mut conn = self.pool.get().await?;
        CategoryDao::resolve_category_names_for_app(&mut conn, app_class, uid).await
    }

    /// Get today's total usage (closed + open) for a specific app.
    pub async fn get_today_usage(&self, app_id: i32, uid: Uid, today: &str) -> anyhow::Result<i64> {
        let mut conn = self.pool.get().await?;
        get_today_app_usage(&mut conn, app_id, uid, today).await
    }

    /// Flush buffered events with delta computation in a single transaction.
    ///
    /// 1. Applies closed-interval deltas from the buffer.
    /// 2. Writes events to the append-only events table.
    /// 3. Refreshes open-interval deltas for all registered uids.
    pub async fn flush_with_deltas(
        &self,
        events: &[TimedEvent],
        event_uids: &[Uid],
        all_uids: &[Uid],
        now: DateTime<Utc>,
    ) -> anyhow::Result<()> {
        use diesel_async::AsyncConnection;

        let mut conn = self.pool.get().await?;
        conn.transaction::<_, anyhow::Error, _>(async |conn| {
            if !events.is_empty() {
                deltas::apply_closed_deltas_from_buffer(conn, events, event_uids, now).await?;
                EventDao::flush_events(conn, events).await?;
            }
            deltas::refresh_open_intervals_inner(conn, all_uids, now).await?;
            Ok(())
        })
        .await
    }
}
