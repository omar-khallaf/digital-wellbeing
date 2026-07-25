//! Delta/interval computation functions for daily usage materialization.
//!
//! Pairs `WindowFocused` open events with close events to produce
//! closed-interval durations, written to both `daily_usage` (per-app)
//! and `daily_usage_by_title` (per-title). Open intervals at end of
//! buffer are tracked for minute-tick accumulation via `increment_open_ms`.
//!
//! ## Sub-modules
//!
//! * `helpers` — individual upsert/query helpers (`upsert_closed_delta`,
//!   `ensure_row_for_open`, `duration_millis`, etc.)
//! * `interval_split` — UTC-day boundary splitting for focus intervals
//! * `pre_buffer` — resolving and closing intervals that started before
//!   the current buffer window
//! * `tests` — unit tests for delta computation

mod helpers;
mod interval_split;
mod pre_buffer;

#[cfg(test)]
mod tests;

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use diesel::{ExpressionMethods, QueryDsl};
use diesel_async::{AsyncConnection, RunQueryDsl};
use wellbeing_core::{
    AppId, Uid, WindowTitle,
    event_types::{CLOSE_EVENT_TYPES, EVENT_WINDOW_FOCUSED, is_close_event_type},
};

use crate::store::schema;

use super::BlockingRepo;
use crate::blocking::buffer::BufferedEvent;

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
            Ok((ts, t)) => {
                // Guard: if a close event was emitted after this focus event,
                // the interval is already closed and must not be re-added to
                // open_millis. This handles missed/desynced close events that
                // leave stale entries in current_focus.
                let already_closed = schema::events::table
                    .filter(schema::events::user_id.eq(uid.0 as i32))
                    .filter(schema::events::event_type.eq_any(CLOSE_EVENT_TYPES))
                    .filter(schema::events::timestamp.gt(ts))
                    .filter(schema::events::timestamp.le(now_millis))
                    .select(schema::events::event_type)
                    .first::<i32>(conn)
                    .await;
                if already_closed.is_ok() {
                    (0, t)
                } else {
                    (Self::duration_millis(ts, now_millis), t)
                }
            }
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

    // ── thin wrapper ────────────────────────────────────────────────

    /// Upsert a closed interval, splitting across UTC-day boundaries.
    pub(super) async fn upsert_closed_delta_for_pair<Conn>(
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
}
