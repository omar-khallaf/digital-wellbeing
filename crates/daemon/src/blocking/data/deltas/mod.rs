//! Delta/interval computation functions for daily usage materialization.
//!
//! Pairs `Focus` open events with close events to produce closed-interval
//! durations, written to both `daily_usage` (per-app) and
//! `daily_usage_by_title` (per-title).  All events carry a uid — there are
//! no system-wide events.
//!
//! # Batch algorithm
//!
//! 1. For each uid in the buffer, fetch the last persisted event from the DB.
//!    If it is a `WindowFocused` event, that interval is still open — prepend it.
//! 2. Group buffer events by uid, sort chronologically, walk focus→next
//!    pairs to compute closed intervals, splitting at UTC day boundaries.
//! 3. Aggregate durations by `(date, uid, app_id)` and
//!    `(date, uid, app_id, title)`, then batch-upsert.
//! 4. The last unpaired focus per uid becomes an open interval → `open_millis`.

mod helpers;
mod pre_buffer;

#[cfg(test)]
mod tests;

use std::collections::HashMap;

use chrono::{DateTime, TimeDelta, Utc};
use diesel_async::AsyncConnection;
use wellbeing_core::{AppId, Uid, WindowTitle, event_types::EVENT_WINDOW_FOCUSED};

use super::repo::BlockingRepo;
use crate::blocking::domain::TimedEvent;

// ── Aggregation key types ────────────────────────────────────────────

#[derive(Hash, PartialEq, Eq, Clone, Debug)]
struct AggKeyApp {
    date: String,
    app_id: AppId,
}

#[derive(Hash, PartialEq, Eq, Clone, Debug)]
struct AggKeyTitle {
    date: String,
    app_id: AppId,
    title: WindowTitle,
}

/// State carried between iterations: the currently-open focus interval.
///
/// Focus events always carry a title — `Option` is not used here.
struct OpenFocus {
    ts_ms: i64,
    app_id: AppId,
    title: WindowTitle,
}

impl BlockingRepo {
    /// Apply closed-interval deltas from a buffered event batch.
    ///
    /// Returns once all deltas have been upserted. Does NOT flush the events
    /// themselves — that is the caller's responsibility.
    pub async fn apply_closed_deltas_from_buffer<Conn>(
        &self,
        conn: &mut Conn,
        events: &[TimedEvent],
        uids: &[Uid],
        now: DateTime<Utc>,
    ) -> anyhow::Result<()>
    where
        Conn: AsyncConnection<Backend = diesel::sqlite::Sqlite>,
    {
        if events.is_empty() {
            return Ok(());
        }

        let now_millis = now.timestamp_millis();

        // ── 1. Fetch last event per uid from DB ─────────────────────
        let last_events = Self::fetch_last_events_for_uids(conn, uids).await;

        // ── 2. Partition buffer events by uid ───────────────────────
        let mut per_uid: HashMap<Uid, Vec<&TimedEvent>> = HashMap::new();
        for timed in events {
            per_uid.entry(timed.event.uid()).or_default().push(timed);
        }

        // ── 3. Aggregate maps ───────────────────────────────────────
        type ClosedAgg = HashMap<(Uid, AggKeyApp), i64>;
        type ClosedAggTitle = HashMap<(Uid, AggKeyTitle), i64>;
        type OpenAgg = HashMap<(Uid, AggKeyApp), i64>;
        type OpenAggTitle = HashMap<(Uid, AggKeyTitle), i64>;

        let mut agg_closed: ClosedAgg = HashMap::new();
        let mut agg_closed_by_title: ClosedAggTitle = HashMap::new();
        let mut agg_open: OpenAgg = HashMap::new();
        let mut agg_open_by_title: OpenAggTitle = HashMap::new();

        // ── 4. Process each uid ────────────────────────────────────
        for (uid, mut uid_events) in per_uid {
            uid_events.sort_by_key(|t| t.timestamp.timestamp_millis());

            let mut open_focus: Option<OpenFocus> = None;

            // 4a. Seed open_focus from the last DB event if it's a Focus.
            if let Some(Some(row)) = last_events.get(&uid)
                && row.event_type == EVENT_WINDOW_FOCUSED
            {
                let app_id = row
                    .app_id
                    .as_deref()
                    .and_then(|s| AppId::new(s).ok())
                    .unwrap_or_else(|| AppId::new("unknown").unwrap());
                let title = row
                    .title
                    .as_deref()
                    .map(WindowTitle::new)
                    .unwrap_or_else(|| WindowTitle::new("(untitled)"));
                open_focus = Some(OpenFocus {
                    ts_ms: row.timestamp,
                    app_id,
                    title,
                });
            }

            // 4b. Walk buffer events chronologically.
            for timed in uid_events {
                let ts_ms = timed.timestamp.timestamp_millis();
                let is_focus = timed.event.event_type() == EVENT_WINDOW_FOCUSED;

                if is_focus {
                    // Close the previous interval.
                    if let Some(prev) = open_focus.take() {
                        accumulate_closed(
                            &prev,
                            ts_ms,
                            uid,
                            &mut agg_closed,
                            &mut agg_closed_by_title,
                        );
                    }
                    open_focus = Some(OpenFocus {
                        ts_ms,
                        app_id: timed
                            .event
                            .app_id()
                            .cloned()
                            .unwrap_or_else(|| AppId::new("unknown").unwrap()),
                        title: timed
                            .event
                            .title()
                            .cloned()
                            .unwrap_or_else(|| WindowTitle::new("(untitled)")),
                    });
                } else if timed.event.is_close_event() {
                    // True termination event — close the open interval.
                    // Idle/resumed are NOT close events; the interval continues.
                    if let Some(prev) = open_focus.take() {
                        accumulate_closed(
                            &prev,
                            ts_ms,
                            uid,
                            &mut agg_closed,
                            &mut agg_closed_by_title,
                        );
                    }
                }
            }

            // 4c. Remaining open focus → open interval.
            if let Some(focus) = open_focus {
                accumulate_open(
                    &focus,
                    now_millis,
                    uid,
                    &mut agg_open,
                    &mut agg_open_by_title,
                );
            }
        }

        // ── 5. Batch-upsert closed deltas ───────────────────────────
        for ((uid, key), delta_ms) in &agg_closed {
            Self::upsert_closed_delta(conn, &key.date, *uid, &key.app_id, *delta_ms).await?;
        }
        for ((uid, key), delta_ms) in &agg_closed_by_title {
            Self::upsert_closed_delta_by_title(
                conn,
                &key.date,
                *uid,
                &key.app_id,
                &key.title,
                *delta_ms,
            )
            .await?;
        }

        // ── 6. Upsert open intervals ────────────────────────────────
        for ((uid, key), open_ms) in &agg_open {
            Self::upsert_open_delta(conn, &key.date, *uid, &key.app_id, *open_ms).await?;
        }
        for ((uid, key), open_ms) in &agg_open_by_title {
            Self::upsert_open_delta_by_title(
                conn,
                &key.date,
                *uid,
                &key.app_id,
                &key.title,
                *open_ms,
            )
            .await?;
        }

        Ok(())
    }

    /// Refresh `open_millis` for registered uids whose last event is an
    /// open focus interval, keeping daily usage up-to-date even when no
    /// focus-switch events arrive between minute ticks.
    ///
    /// Called by `flush_buffer()` on every minute tick.
    pub(crate) async fn refresh_open_intervals<Conn>(
        conn: &mut Conn,
        uids: &[Uid],
        now: DateTime<Utc>,
    ) -> anyhow::Result<()>
    where
        Conn: AsyncConnection<Backend = diesel::sqlite::Sqlite>,
    {
        let now_millis = now.timestamp_millis();
        let last_events = Self::fetch_last_events_for_uids(conn, uids).await;

        let mut agg_open: HashMap<(Uid, AggKeyApp), i64> = HashMap::new();
        let mut agg_open_by_title: HashMap<(Uid, AggKeyTitle), i64> = HashMap::new();

        for (&uid, row) in &last_events {
            let Some(row) = row else { continue };
            if row.event_type != EVENT_WINDOW_FOCUSED {
                continue;
            }
            if now_millis <= row.timestamp {
                continue;
            }

            let app_id = row
                .app_id
                .as_deref()
                .and_then(|s| AppId::new(s).ok())
                .unwrap_or_else(|| AppId::new("unknown").unwrap());
            let title = row
                .title
                .as_deref()
                .map(WindowTitle::new)
                .unwrap_or_else(|| WindowTitle::new("(untitled)"));

            let focus = OpenFocus {
                ts_ms: row.timestamp,
                app_id,
                title,
            };

            accumulate_open(
                &focus,
                now_millis,
                uid,
                &mut agg_open,
                &mut agg_open_by_title,
            );
        }

        for ((uid, key), open_ms) in &agg_open {
            Self::upsert_open_delta(conn, &key.date, *uid, &key.app_id, *open_ms).await?;
        }
        for ((uid, key), open_ms) in &agg_open_by_title {
            Self::upsert_open_delta_by_title(
                conn,
                &key.date,
                *uid,
                &key.app_id,
                &key.title,
                *open_ms,
            )
            .await?;
        }

        Ok(())
    }
}

// ── Accumulation helpers ─────────────────────────────────────────────

/// Accumulate a closed interval from `focus` → `next_ts`, splitting at UTC
/// midnight boundaries so each day segment is aggregated under the correct
/// date key. No synthetic events are created — the split is purely in the
/// duration computation.
fn accumulate_closed(
    focus: &OpenFocus,
    next_ts: i64,
    uid: Uid,
    agg_closed: &mut HashMap<(Uid, AggKeyApp), i64>,
    agg_closed_by_title: &mut HashMap<(Uid, AggKeyTitle), i64>,
) {
    let mut seg_start = focus.ts_ms;

    loop {
        let seg_start_dt =
            DateTime::from_timestamp_millis(seg_start).expect("valid event timestamp");
        let next_boundary = (seg_start_dt.date_naive() + TimeDelta::days(1))
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc();
        let next_boundary_ms = next_boundary.timestamp_millis();

        let date = seg_start_dt.format("%Y-%m-%d").to_string();
        let dur = if next_boundary_ms >= next_ts {
            (next_ts - seg_start).max(0)
        } else {
            (next_boundary_ms - seg_start).max(0)
        };

        *agg_closed
            .entry((
                uid,
                AggKeyApp {
                    date: date.clone(),
                    app_id: focus.app_id.clone(),
                },
            ))
            .or_insert(0) += dur;
        *agg_closed_by_title
            .entry((
                uid,
                AggKeyTitle {
                    date,
                    app_id: focus.app_id.clone(),
                    title: focus.title.clone(),
                },
            ))
            .or_insert(0) += dur;

        if next_boundary_ms >= next_ts {
            break;
        }
        seg_start = next_boundary_ms;
    }
}

/// Accumulate an open interval from `focus` → `now`, splitting at midnight.
fn accumulate_open(
    focus: &OpenFocus,
    now_ms: i64,
    uid: Uid,
    agg_open: &mut HashMap<(Uid, AggKeyApp), i64>,
    agg_open_by_title: &mut HashMap<(Uid, AggKeyTitle), i64>,
) {
    if now_ms <= focus.ts_ms {
        return;
    }

    let mut seg_start = focus.ts_ms;

    loop {
        let seg_start_dt =
            DateTime::from_timestamp_millis(seg_start).expect("valid event timestamp");
        let next_boundary = (seg_start_dt.date_naive() + TimeDelta::days(1))
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc();
        let next_boundary_ms = next_boundary.timestamp_millis();

        let date = seg_start_dt.format("%Y-%m-%d").to_string();
        let dur = if next_boundary_ms >= now_ms {
            (now_ms - seg_start).max(0)
        } else {
            (next_boundary_ms - seg_start).max(0)
        };

        agg_open.insert(
            (
                uid,
                AggKeyApp {
                    date: date.clone(),
                    app_id: focus.app_id.clone(),
                },
            ),
            dur,
        );
        agg_open_by_title.insert(
            (
                uid,
                AggKeyTitle {
                    date,
                    app_id: focus.app_id.clone(),
                    title: focus.title.clone(),
                },
            ),
            dur,
        );

        if next_boundary_ms >= now_ms {
            break;
        }
        seg_start = next_boundary_ms;
    }
}
