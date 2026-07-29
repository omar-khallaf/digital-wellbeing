//! Delta/interval computation for daily usage materialization.
//!
//! Single-pass entry point [`process_all_deltas`] handles both closed-interval
//! accumulation (from buffered events) and open-interval refresh (for all
//! registered UIDs) in one go.  All upserts use batch multi-row statements.
//!
//! Flow:
//! 1. Accept a pre-fetched `last_events` map (caller fetches once).
//! 2. Group buffer events by UID, sort chronologically.
//! 3. For UIDs with events: seed `open_focus` from last DB event, walk
//!    buffer events to close/accumulate intervals, finalise open interval.
//! 4. For UIDs without events: derive open interval from `last_events`.
//! 5. Accumulate category deltas from the (uid, app_id) aggregates.
//! 6. Batch-upsert into all six daily_usage tables.

use std::collections::{BTreeSet, HashMap, HashSet};

use chrono::{DateTime, Utc};
use wellbeing_core::{AppClass, EventType, Uid, WindowTitle};

use crate::blocking::domain::TimedEvent;
use crate::store::connection::DbConn;
use crate::store::daos::events::EventRow;

use super::accumulate::{accumulate_closed, accumulate_open};
use super::upserts::*;
use crate::store::daos::apps::AppsDao;

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

#[derive(Hash, PartialEq, Eq, Clone, Debug)]
pub(crate) struct AggKeyApp {
    pub(crate) date: String,
    pub(crate) app_id: i32,
}

#[derive(Hash, PartialEq, Eq, Clone, Debug)]
pub(crate) struct AggKeyCategory {
    pub(crate) date: String,
    pub(crate) category_id: i32,
}

#[derive(Hash, PartialEq, Eq, Clone, Debug)]
pub(crate) struct AggKeyTitle {
    pub(crate) date: String,
    pub(crate) app_class: String,
    pub(crate) app_id: i32,
    pub(crate) title: WindowTitle,
}

/// State carried between iterations: the currently-open focus interval.
pub(crate) struct OpenFocus {
    pub(crate) ts_ms: i64,
    pub(crate) app_class: AppClass,
    pub(crate) app_id: i32,
    pub(crate) title: WindowTitle,
}

// ---------------------------------------------------------------------------
// Category delta accumulation (shared — already efficient, one SELECT)
// ---------------------------------------------------------------------------

/// From app-aggregated deltas, resolve categories and fan out into per-category
/// aggregates.
async fn accumulate_category_deltas(
    conn: &DbConn,
    agg: &HashMap<(Uid, AggKeyApp), i64>,
    agg_category: &mut HashMap<(Uid, AggKeyCategory), i64>,
) -> anyhow::Result<()> {
    if agg.is_empty() {
        return Ok(());
    }
    let mut pairs: Vec<(Uid, i32)> = Vec::new();
    let mut seen: BTreeSet<(Uid, i32)> = BTreeSet::new();
    for (uid, key) in agg.keys() {
        if seen.insert((*uid, key.app_id)) {
            pairs.push((*uid, key.app_id));
        }
    }
    let cat_map =
        crate::store::daos::categories::CategoryDao::resolve_category_map(conn, &pairs).await;
    for ((uid, key), delta_ms) in agg {
        if let Some(cat_ids) = cat_map.get(&(*uid, key.app_id)) {
            for &cat_id in cat_ids {
                *agg_category
                    .entry((
                        *uid,
                        AggKeyCategory {
                            date: key.date.clone(),
                            category_id: cat_id,
                        },
                    ))
                    .or_insert(0) += *delta_ms;
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// ensure_app with per-tick cache
// ---------------------------------------------------------------------------

/// Thin cache keyed by `AppClass` so repeated calls for the same app in a
/// single tick hit only the first round-trip.
async fn cached_ensure_app(
    conn: &DbConn,
    app_class: &AppClass,
    cache: &mut HashMap<AppClass, i32>,
) -> i32 {
    if let Some(&id) = cache.get(app_class) {
        return id;
    }
    let id = AppsDao::ensure_app(conn, app_class).await.unwrap_or(0);
    cache.insert(app_class.clone(), id);
    id
}

// ---------------------------------------------------------------------------
// OpenFocus builders
// ---------------------------------------------------------------------------

/// Build an `OpenFocus` from a persisted `EventRow` (last event from DB).
async fn open_focus_from_row(
    conn: &DbConn,
    row: &EventRow,
    cache: &mut HashMap<AppClass, i32>,
) -> Option<OpenFocus> {
    let event_type = EventType::try_from(row.event_type).unwrap_or(EventType::Focus);
    if event_type != EventType::Focus {
        return None;
    }
    let app_class = row
        .app_class
        .as_deref()
        .and_then(|s| AppClass::new(s).ok())
        .unwrap_or_else(|| AppClass::new("unknown").unwrap());
    let app_id = cached_ensure_app(conn, &app_class, cache).await;
    let title = row
        .title
        .as_deref()
        .map(WindowTitle::new)
        .unwrap_or_else(|| WindowTitle::new("(untitled)"));
    Some(OpenFocus {
        ts_ms: row.timestamp,
        app_class,
        app_id,
        title,
    })
}

/// Build an `OpenFocus` from a buffered `TimedEvent`.
async fn open_focus_from_event(
    conn: &DbConn,
    timed: &TimedEvent,
    cache: &mut HashMap<AppClass, i32>,
) -> OpenFocus {
    let ts_ms = timed.timestamp.timestamp_millis();
    let app_class = timed
        .event
        .app_class()
        .cloned()
        .unwrap_or_else(|| AppClass::new("unknown").unwrap());
    let app_id = cached_ensure_app(conn, &app_class, cache).await;
    let title = timed
        .event
        .title()
        .cloned()
        .unwrap_or_else(|| WindowTitle::new("(untitled)"));
    OpenFocus {
        ts_ms,
        app_class,
        app_id,
        title,
    }
}

// ---------------------------------------------------------------------------
// Single entry point
// ---------------------------------------------------------------------------

/// Single-pass delta computation and persistence.
///
/// # Arguments
/// * `conn`        — database connection (inside a transaction)
/// * `events`      — buffered events to process for closed intervals
/// * `all_uids`    — all registered UIDs (for open-interval refresh)
/// * `last_events` — pre-fetched `get_last_events_for_uids(all_uids)` result
/// * `now`         — wall clock for open-interval durations
pub(crate) async fn process_all_deltas(
    conn: &DbConn,
    events: &[TimedEvent],
    all_uids: &[Uid],
    last_events: &HashMap<Uid, Option<EventRow>>,
    now: DateTime<Utc>,
    app_cache: &mut HashMap<AppClass, i32>,
) -> anyhow::Result<()> {
    let now_millis = now.timestamp_millis();

    // --- accumulators ---
    let mut agg_closed: HashMap<(Uid, AggKeyApp), i64> = HashMap::new();
    let mut agg_closed_by_title: HashMap<(Uid, AggKeyTitle), i64> = HashMap::new();
    let mut agg_closed_category: HashMap<(Uid, AggKeyCategory), i64> = HashMap::new();
    let mut agg_open: HashMap<(Uid, AggKeyApp), i64> = HashMap::new();
    let mut agg_open_by_title: HashMap<(Uid, AggKeyTitle), i64> = HashMap::new();
    let mut agg_open_category: HashMap<(Uid, AggKeyCategory), i64> = HashMap::new();

    // --- Phase A: UIDs that have buffered events ---

    let mut per_uid: HashMap<Uid, Vec<&TimedEvent>> = HashMap::new();
    for timed in events {
        per_uid.entry(timed.event.uid()).or_default().push(timed);
    }

    for (uid, mut uid_events) in per_uid {
        uid_events.sort_by_key(|t| t.timestamp.timestamp_millis());

        let mut open_focus: Option<OpenFocus> = None;

        // Seed from last persisted event, if it was a Focus.
        if let Some(Some(row)) = last_events.get(&uid) {
            open_focus = open_focus_from_row(conn, row, &mut *app_cache).await;
        }

        for timed in &uid_events {
            let ts_ms = timed.timestamp.timestamp_millis();
            let is_focus = timed.event.event_type() == EventType::Focus;

            if is_focus {
                // Close the previous interval if one was open.
                if let Some(prev) = open_focus.take() {
                    accumulate_closed(&prev, ts_ms, uid, &mut agg_closed, &mut agg_closed_by_title);
                }
                open_focus = Some(open_focus_from_event(conn, timed, &mut *app_cache).await);
            } else if timed.event.is_close_event()
                && let Some(prev) = open_focus.take()
            {
                accumulate_closed(&prev, ts_ms, uid, &mut agg_closed, &mut agg_closed_by_title);
            }
        }

        // Remaining open focus → open interval.
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

    // --- Phase B: UIDs without buffered events — just refresh open intervals ---

    let event_uids: HashSet<Uid> = events.iter().map(|e| e.event.uid()).collect();

    for &uid in all_uids {
        if event_uids.contains(&uid) {
            continue;
        }
        let Some(Some(row)) = last_events.get(&uid) else {
            continue;
        };
        let Some(focus) = open_focus_from_row(conn, row, &mut *app_cache).await else {
            continue;
        };
        accumulate_open(
            &focus,
            now_millis,
            uid,
            &mut agg_open,
            &mut agg_open_by_title,
        );
    }

    // --- Category accumulation ---

    accumulate_category_deltas(conn, &agg_closed, &mut agg_closed_category).await?;
    accumulate_category_deltas(conn, &agg_open, &mut agg_open_category).await?;

    // --- Batch upserts ---

    let closed_apps: Vec<ClosedAppItem> = agg_closed
        .into_iter()
        .map(|((uid, key), delta_ms)| ClosedAppItem {
            date: key.date,
            uid,
            app_id: key.app_id,
            delta_ms,
        })
        .collect();
    batch_upsert_closed_apps(conn, &closed_apps).await?;

    let closed_titles: Vec<ClosedTitleItem> = agg_closed_by_title
        .into_iter()
        .map(|((uid, key), delta_ms)| ClosedTitleItem {
            date: key.date,
            uid,
            app_id: key.app_id,
            // app_class stored inside key but app_id is already resolved,
            // so we can pass it directly.  The title upsert needs no
            // separate app_class → app_id lookup.
            title: key.title,
            delta_ms,
        })
        .collect();
    batch_upsert_closed_titles(conn, &closed_titles).await?;

    let closed_categories: Vec<ClosedCategoryItem> = agg_closed_category
        .into_iter()
        .map(|((uid, key), delta_ms)| ClosedCategoryItem {
            date: key.date,
            uid,
            category_id: key.category_id,
            delta_ms,
        })
        .collect();
    batch_upsert_closed_categories(conn, &closed_categories).await?;

    let open_apps: Vec<OpenAppItem> = agg_open
        .into_iter()
        .map(|((uid, key), open_ms)| OpenAppItem {
            date: key.date,
            uid,
            app_id: key.app_id,
            open_ms,
        })
        .collect();
    batch_upsert_open_apps(conn, &open_apps).await?;

    let open_titles: Vec<OpenTitleItem> = agg_open_by_title
        .into_iter()
        .map(|((uid, key), open_ms)| OpenTitleItem {
            date: key.date,
            uid,
            app_id: key.app_id,
            title: key.title,
            open_ms,
        })
        .collect();
    batch_upsert_open_titles(conn, &open_titles).await?;

    let open_categories: Vec<OpenCategoryItem> = agg_open_category
        .into_iter()
        .map(|((uid, key), open_ms)| OpenCategoryItem {
            date: key.date,
            uid,
            category_id: key.category_id,
            open_ms,
        })
        .collect();
    batch_upsert_open_categories(conn, &open_categories).await?;

    Ok(())
}
