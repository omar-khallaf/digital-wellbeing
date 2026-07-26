//! Delta/interval computation for daily usage materialization.
//!
//! 1. Fetch last persisted event per uid from DB; if it's a `Focus`,
//!    that interval is still open — prepend it as open_focus.
//! 2. Group buffer events by uid, sort chronologically, walk focus->next
//!    pairs to compute closed intervals, splitting at UTC day boundaries.
//! 3. Aggregate durations by `(date, uid, app_id)` and
//!    `(date, uid, app_class, title)`, then batch-upsert.
//! 4. The last unpaired focus per uid becomes an open interval -> `open_millis`.

use std::collections::{BTreeSet, HashMap};

use chrono::{DateTime, Utc};
use wellbeing_core::{EventType, Uid, WindowTitle};

use crate::blocking::domain::TimedEvent;
use crate::store::connection::DbConn;

use super::accumulate::{accumulate_closed, accumulate_open};
use super::upserts::*;
use crate::store::daos::apps::AppsDao;

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
    pub(crate) title: WindowTitle,
}

/// State carried between iterations: the currently-open focus interval.
pub(crate) struct OpenFocus {
    pub(crate) ts_ms: i64,
    pub(crate) app_class: wellbeing_core::AppClass,
    pub(crate) app_id: i32,
    pub(crate) title: WindowTitle,
}

/// Accumulate category deltas from app-aggregated (uid, AggKeyApp) deltas.
async fn accumulate_category_deltas(
    conn: &mut DbConn,
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

/// Apply closed-interval deltas from a buffered event batch.
pub(crate) async fn apply_closed_deltas_from_buffer(
    conn: &mut DbConn,
    events: &[TimedEvent],
    uids: &[Uid],
    now: DateTime<Utc>,
) -> anyhow::Result<()> {
    if events.is_empty() {
        return Ok(());
    }

    let now_millis = now.timestamp_millis();

    // 1. Fetch last event per uid from DB.
    let last_events =
        crate::store::daos::events::EventDao::get_last_events_for_uids(conn, uids).await;

    let mut per_uid: HashMap<Uid, Vec<&TimedEvent>> = HashMap::new();
    for timed in events {
        per_uid.entry(timed.event.uid()).or_default().push(timed);
    }

    type ClosedAgg = HashMap<(Uid, AggKeyApp), i64>;
    type ClosedAggTitle = HashMap<(Uid, AggKeyTitle), i64>;
    type ClosedAggCategory = HashMap<(Uid, AggKeyCategory), i64>;
    type OpenAgg = HashMap<(Uid, AggKeyApp), i64>;
    type OpenAggTitle = HashMap<(Uid, AggKeyTitle), i64>;
    type OpenAggCategory = HashMap<(Uid, AggKeyCategory), i64>;

    let mut agg_closed: ClosedAgg = HashMap::new();
    let mut agg_closed_by_title: ClosedAggTitle = HashMap::new();
    let mut agg_closed_category: ClosedAggCategory = HashMap::new();
    let mut agg_open: OpenAgg = HashMap::new();
    let mut agg_open_by_title: OpenAggTitle = HashMap::new();
    let mut agg_open_category: OpenAggCategory = HashMap::new();

    for (uid, mut uid_events) in per_uid {
        uid_events.sort_by_key(|t| t.timestamp.timestamp_millis());

        let mut open_focus: Option<OpenFocus> = None;

        // 4a. Seed open_focus from the last DB event if it's a Focus.
        // Convert raw i32 to domain type at the boundary.
        if let Some(Some(row)) = last_events.get(&uid) {
            let event_type = EventType::try_from(row.event_type).unwrap_or(EventType::Focus);
            if event_type == EventType::Focus {
                let app_class = row
                    .app_class
                    .as_deref()
                    .and_then(|s| wellbeing_core::AppClass::new(s).ok())
                    .unwrap_or_else(|| wellbeing_core::AppClass::new("unknown").unwrap());
                let app_id = AppsDao::ensure_app(conn, &app_class).await.unwrap_or(0);
                let title = row
                    .title
                    .as_deref()
                    .map(WindowTitle::new)
                    .unwrap_or_else(|| WindowTitle::new("(untitled)"));
                open_focus = Some(OpenFocus {
                    ts_ms: row.timestamp,
                    app_class,
                    app_id,
                    title,
                });
            }
        }

        for timed in uid_events {
            let ts_ms = timed.timestamp.timestamp_millis();
            let is_focus = timed.event.event_type() == EventType::Focus;

            if is_focus {
                if let Some(prev) = open_focus.take() {
                    accumulate_closed(&prev, ts_ms, uid, &mut agg_closed, &mut agg_closed_by_title);
                }
                let app_class = timed
                    .event
                    .app_class()
                    .cloned()
                    .unwrap_or_else(|| wellbeing_core::AppClass::new("unknown").unwrap());
                let app_id = AppsDao::ensure_app(conn, &app_class).await.unwrap_or(0);
                open_focus = Some(OpenFocus {
                    ts_ms,
                    app_class,
                    app_id,
                    title: timed
                        .event
                        .title()
                        .cloned()
                        .unwrap_or_else(|| WindowTitle::new("(untitled)")),
                });
            } else if timed.event.is_close_event()
                && let Some(prev) = open_focus.take()
            {
                accumulate_closed(&prev, ts_ms, uid, &mut agg_closed, &mut agg_closed_by_title);
            }
        }

        // 4c. Remaining open focus -> open interval.
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

    for ((uid, key), delta_ms) in &agg_closed {
        upsert_closed_delta(conn, &key.date, *uid, key.app_id, *delta_ms).await?;
    }
    for ((uid, key), delta_ms) in &agg_closed_by_title {
        upsert_closed_delta_by_title(conn, &key.date, *uid, &key.app_class, &key.title, *delta_ms)
            .await?;
    }

    accumulate_category_deltas(conn, &agg_closed, &mut agg_closed_category).await?;

    for ((uid, key), delta_ms) in &agg_closed_category {
        upsert_closed_delta_by_category(conn, &key.date, *uid, key.category_id, *delta_ms).await?;
    }

    for ((uid, key), open_ms) in &agg_open {
        upsert_open_delta(conn, &key.date, *uid, key.app_id, *open_ms).await?;
    }
    for ((uid, key), open_ms) in &agg_open_by_title {
        upsert_open_delta_by_title(conn, &key.date, *uid, &key.app_class, &key.title, *open_ms)
            .await?;
    }

    accumulate_category_deltas(conn, &agg_open, &mut agg_open_category).await?;
    for ((uid, key), open_ms) in &agg_open_category {
        upsert_open_delta_by_category(conn, &key.date, *uid, key.category_id, *open_ms).await?;
    }

    Ok(())
}

/// Refresh `open_millis` for registered uids (minute-tick).
pub(crate) async fn refresh_open_intervals_inner(
    conn: &mut DbConn,
    uids: &[Uid],
    now: DateTime<Utc>,
) -> anyhow::Result<()> {
    let now_millis = now.timestamp_millis();
    let last_events =
        crate::store::daos::events::EventDao::get_last_events_for_uids(conn, uids).await;

    let mut agg_open: HashMap<(Uid, AggKeyApp), i64> = HashMap::new();
    let mut agg_open_by_title: HashMap<(Uid, AggKeyTitle), i64> = HashMap::new();
    let mut agg_open_category: HashMap<(Uid, AggKeyCategory), i64> = HashMap::new();

    for (&uid, row) in &last_events {
        let Some(row) = row else { continue };
        let event_type = EventType::try_from(row.event_type).unwrap_or(EventType::Focus);
        if event_type != EventType::Focus {
            continue;
        }
        if now_millis <= row.timestamp {
            continue;
        }

        let app_class = row
            .app_class
            .as_deref()
            .and_then(|s| wellbeing_core::AppClass::new(s).ok())
            .unwrap_or_else(|| wellbeing_core::AppClass::new("unknown").unwrap());
        let app_id = AppsDao::ensure_app(conn, &app_class).await.unwrap_or(0);
        let title = row
            .title
            .as_deref()
            .map(WindowTitle::new)
            .unwrap_or_else(|| WindowTitle::new("(untitled)"));

        let focus = OpenFocus {
            ts_ms: row.timestamp,
            app_class,
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
        upsert_open_delta(conn, &key.date, *uid, key.app_id, *open_ms).await?;
    }
    for ((uid, key), open_ms) in &agg_open_by_title {
        upsert_open_delta_by_title(conn, &key.date, *uid, &key.app_class, &key.title, *open_ms)
            .await?;
    }

    accumulate_category_deltas(conn, &agg_open, &mut agg_open_category).await?;
    for ((uid, key), open_ms) in &agg_open_category {
        upsert_open_delta_by_category(conn, &key.date, *uid, key.category_id, *open_ms).await?;
    }

    Ok(())
}
