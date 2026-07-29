//! Repository for blocking/enforcement persistence operations.
//!
//! Wraps store DAO calls and provides coarse-grained methods for the
//! [`EnforcerActor`](super::super::core::EnforcerActor).

use std::collections::HashMap;

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
        let conn = self.pool.client().await?;
        AppsDao::ensure_app(&conn, app_class).await
    }

    /// Resolve category discriminants for an app (user-specific then system fallback).
    /// Returns `Vec<i32>` where each i32 is the `#[repr(u8)]` value of the
    /// [`Category`](wellbeing_core::Category) enum (0=Productivity…6=Uncategorized).
    pub async fn resolve_category_discriminants(
        &self,
        app_class: &AppClass,
        uid: Uid,
    ) -> anyhow::Result<Vec<i32>> {
        let conn = self.pool.client().await?;
        let categories = CategoryDao::resolve_categories_for_app(&conn, app_class, uid).await?;
        Ok(categories.into_iter().map(|c| c as i32).collect())
    }

    /// Get today's total usage (closed + open) for a specific app.
    pub async fn get_today_usage(&self, app_id: i32, uid: Uid, today: &str) -> anyhow::Result<i64> {
        let conn = self.pool.client().await?;
        get_today_app_usage(&conn, app_id, uid, today).await
    }

    /// Flush buffered events with delta computation in a single transaction.
    ///
    /// 1. Fetches `last_events` for all registered UIDs (one round-trip).
    /// 2. Merged pass: closed intervals from buffered events + open intervals
    ///    for all UIDs — in one go.
    /// 3. Batch-inserts buffered events.
    /// 4. Commits.
    ///
    /// `app_cache` is populated during processing and can be reused by the
    /// caller for policy evaluation, saving redundant `ensure_app` calls.
    pub async fn flush_with_deltas(
        &self,
        events: &[TimedEvent],
        all_uids: &[Uid],
        now: DateTime<Utc>,
        app_cache: &mut HashMap<AppClass, i32>,
    ) -> anyhow::Result<()> {
        let mut conn = self.pool.client().await?;
        let tx = conn.transaction().await?;

        // Single fetch for the entire tick.
        let last_events = EventDao::get_last_events_for_uids(&tx, all_uids).await?;

        // Merged pass: closed (from buffer) + open (all UIDs).
        deltas::process_all_deltas(&tx, events, all_uids, &last_events, now, app_cache).await?;

        if !events.is_empty() {
            EventDao::batch_insert_events(&tx, events).await?;
        }

        tx.commit().await?;
        Ok(())
    }
}
