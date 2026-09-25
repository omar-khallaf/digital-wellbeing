//! Day timeline and hourly bucket computations.
//!
//! Converts raw D-Bus event rows into focus interval blocks and hourly
//! breakdowns for the timeline view. Pure functions — no gpui, no async.

use std::collections::HashMap;

use chrono::{DateTime, NaiveDate, Utc};
use wellbeing_core::{AppClass, DayEventRow, EventType, clock::split_by_utc_day};

use super::domain::{DayTimeline, HourlyBucket, TimelineBlock, TimelineFragment};

fn ms_to_dt(ts: i64) -> DateTime<Utc> {
    DateTime::from_timestamp_millis(ts)
        .unwrap_or_else(|| panic!("valid epoch millis in event row, got {ts}"))
}

fn display_name(app_class: &str, names: &HashMap<String, String>) -> String {
    names
        .get(app_class)
        .cloned()
        .unwrap_or_else(|| app_class.to_string())
}

fn make_focus_block(
    start_ts: i64,
    end_ts: i64,
    app_class: &str,
    names: &HashMap<String, String>,
    event_type: EventType,
) -> TimelineBlock {
    TimelineBlock {
        app_class: app_class.to_string(),
        display_name: display_name(app_class, names),
        start: ms_to_dt(start_ts),
        end: Some(ms_to_dt(end_ts)),
        event_type,
        is_gap: false,
    }
}

fn fill_time_gaps(blocks: &mut Vec<TimelineBlock>) {
    if blocks.is_empty() {
        return;
    }
    let mut i = 0;
    while i < blocks.len() - 1 {
        if let Some(prev_end) = blocks[i].end {
            let next_start = blocks[i + 1].start;
            if prev_end < next_start {
                let gap = TimelineBlock {
                    app_class: String::new(),
                    display_name: String::new(),
                    start: prev_end,
                    end: Some(next_start),
                    event_type: EventType::Block,
                    is_gap: true,
                };
                blocks.insert(i + 1, gap);
                i += 1;
            }
        }
        i += 1;
    }
}

/// Build a `DayTimeline` from raw D-Bus event rows for a single day.
///
/// Focus opens an interval; `EventType::is_close` terminates one.
/// Resume and other event types are ignored.
///
/// `seed` is the trailing pre-midnight Focus — the last Focus strictly before
/// `day_start` with no close/Idle after it — as `(timestamp_millis,
/// app_class)`. It is trusted regardless of age (no staleness check) and
/// clipped to `day_start`, so when seeded and no in-range event precedes the
/// first close, a block `[day_start, first_close)` is emitted for the seeded
/// app. `None` leaves unseeded behavior unchanged.
///
/// `now` is used to compute the duration of the currently-open block (if any)
/// so `total_focus_millis` includes the active interval rather than ignoring it.
///
/// Note: the daemon already returns events sorted by `timestamp ASC` (the SQL
/// query has `ORDER BY timestamp ASC`) so no re-sort is needed here.
pub fn build_day_timeline(
    events: &mut [DayEventRow],
    date: NaiveDate,
    app_names: &HashMap<String, String>,
    now: DateTime<Utc>,
    seed: Option<(i64, AppClass)>,
) -> DayTimeline {
    // Events are already sorted by timestamp ASC from the daemon SQL query.
    #[cfg(debug_assertions)]
    debug_assert!(
        events.windows(2).all(|w| w[0].timestamp <= w[1].timestamp),
        "day_events must be pre-sorted by timestamp ASC"
    );

    let day_start_ms = date
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_utc()
        .timestamp_millis();

    let mut blocks: Vec<TimelineBlock> = Vec::new();
    let mut pending_focus: Option<(i64, String)> = seed
        .filter(|(ts, _)| *ts < day_start_ms)
        .map(|(_, app)| (day_start_ms, app.as_ref().to_string()));

    for event in events.iter() {
        match event.event_type {
            EventType::Focus => {
                if let Some((st, ref aid)) = pending_focus.take() {
                    blocks.push(make_focus_block(
                        st,
                        event.timestamp,
                        aid,
                        app_names,
                        EventType::Focus,
                    ));
                }
                pending_focus = Some((event.timestamp, event.app_class.to_string()));
            }
            e if e.is_close() => {
                if let Some((st, ref aid)) = pending_focus.take() {
                    blocks.push(make_focus_block(st, event.timestamp, aid, app_names, e));
                }
            }
            _ => {}
        }
    }

    if let Some((st, ref aid)) = pending_focus.take() {
        blocks.push(TimelineBlock {
            app_class: aid.clone(),
            display_name: display_name(aid, app_names),
            start: ms_to_dt(st),
            end: None,
            event_type: EventType::Focus,
            is_gap: false,
        });
    }

    blocks.sort_by_key(|b| b.start);

    fill_time_gaps(&mut blocks);

    let now_ms = now.timestamp_millis();

    let total_focus_millis: i64 = blocks
        .iter()
        .filter(|b| !b.is_gap)
        .map(|b| {
            let end = b.end.map(|e| e.timestamp_millis()).unwrap_or(now_ms);
            (end - b.start.timestamp_millis()).max(0)
        })
        .sum();

    DayTimeline {
        date,
        blocks,
        total_focus_millis,
    }
}

/// Intersect `[start_ms, end_ms)` with the UTC day opening at `day_start`.
///
/// Splits cross-midnight intervals via `split_by_utc_day` and keeps only this
/// day's share, so a block spanning midnight contributes its pre-midnight
/// millis here and its post-midnight millis to the next day — never both.
fn day_segment(start_ms: i64, end_ms: i64, day_start: i64) -> Option<(i64, i64)> {
    for (seg_day, dur) in split_by_utc_day(start_ms, end_ms) {
        if seg_day == day_start {
            let seg_start = start_ms.max(day_start);
            return Some((seg_start, seg_start + dur));
        }
    }
    None
}

/// Build 24 hourly buckets from a `DayTimeline`.
///
/// Each bucket covers one clock hour (0-23). All blocks (both focus
/// intervals and gap blocks from `fill_time_gaps`) are converted into
/// `TimelineFragment` values sorted by their absolute start offset within
/// the hour, so the renderer can position each fragment at the correct
/// time — whether at the start of the hour, in the middle, or offset by
/// idle time before the first event.
///
/// `now` is used to resolve open-ended blocks (currently-open focus
/// intervals) so they contribute elapsed time to the correct hours.
/// Pass the current UTC time at the point of rendering.
pub fn compute_hourly_buckets(timeline: &DayTimeline, now: DateTime<Utc>) -> Vec<HourlyBucket> {
    let mut buckets: Vec<HourlyBucket> = (0..24)
        .map(|hour| HourlyBucket {
            hour,
            fragments: Vec::new(),
            total_millis: 0,
        })
        .collect();

    let now_ms = now.timestamp_millis();

    let hour_starts: Vec<i64> = (0..24)
        .map(|h| {
            timeline
                .date
                .and_hms_opt(h, 0, 0)
                .unwrap()
                .and_utc()
                .timestamp_millis()
        })
        .collect();

    let day_start = hour_starts[0];

    for block in &timeline.blocks {
        let start_ms = block.start.timestamp_millis();
        let end_ms = block.end.map(|e| e.timestamp_millis()).unwrap_or_else(|| {
            // Open block (currently-focused app with no close event yet).
            // Use `now` so the ongoing interval contributes elapsed time
            // to the correct hours instead of a synthetic 1ms in the
            // start hour only.
            now_ms.max(start_ms + 1)
        });

        let Some((start_ms, end_ms)) = day_segment(start_ms, end_ms, day_start) else {
            continue;
        };

        let start_hour = (((start_ms - day_start) / 3_600_000) % 24).max(0) as usize;
        let end_hour = (((end_ms - 1 - day_start) / 3_600_000).clamp(0, 23)) as usize;

        for h in start_hour..=end_hour {
            let actual_hour_start = hour_starts[h];
            let hour_start = actual_hour_start.max(start_ms);
            let hour_end = if h == 23 {
                day_start + 86_400_000
            } else {
                hour_starts[h + 1]
            };
            let overlap_end = end_ms.min(hour_end);
            let millis = (overlap_end - hour_start).max(0);

            if millis == 0 {
                continue;
            }

            let start_offset = hour_start - actual_hour_start;
            buckets[h].fragments.push(TimelineFragment {
                app_class: block.app_class.clone(),
                display_name: block.display_name.clone(),
                millis,
                is_gap: block.is_gap,
                start_offset,
            });
            buckets[h].total_millis += millis;
        }
    }

    for bucket in &mut buckets {
        bucket.fragments.sort_by_key(|f| f.start_offset);
    }

    buckets
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use wellbeing_core::AppClass;

    fn make_row(event_type: EventType, timestamp: i64, app_class: &str) -> DayEventRow {
        DayEventRow {
            id: 0,
            event_type,
            timestamp,
            app_class: AppClass::new(app_class).unwrap(),
            title: wellbeing_core::WindowTitle::new(""),
            user_id: 0,
        }
    }

    fn test_now() -> DateTime<Utc> {
        DateTime::from_timestamp_millis(1_748_773_800_000).unwrap() // 10:30:00 UTC
    }

    #[test]
    fn test_empty_events_returns_empty_timeline() {
        let date = NaiveDate::from_ymd_opt(2025, 6, 1).unwrap();
        let names = HashMap::new();
        let tl = build_day_timeline(&mut [], date, &names, test_now(), None);
        assert!(tl.blocks.is_empty());
        assert_eq!(tl.total_focus_millis, 0);
        assert_eq!(tl.date, date);
    }

    #[test]
    fn test_single_focus_with_close() {
        let date = NaiveDate::from_ymd_opt(2025, 6, 1).unwrap();
        let names = HashMap::new();
        let mut events = vec![
            make_row(EventType::Focus, 1_748_772_000_000, "org.mozilla.Firefox"),
            make_row(EventType::Unfocus, 1_748_772_050_000, "org.mozilla.Firefox"),
        ];
        let tl = build_day_timeline(&mut events, date, &names, test_now(), None);
        assert_eq!(tl.blocks.len(), 1);
        assert!(!tl.blocks[0].is_gap);
        assert_eq!(tl.blocks[0].app_class, "org.mozilla.Firefox");
        assert!(tl.blocks[0].end.is_some());
        assert_eq!(tl.total_focus_millis, 50_000);
    }

    #[test]
    fn test_unmatched_focus_at_end() {
        let date = NaiveDate::from_ymd_opt(2025, 6, 1).unwrap();
        let names = HashMap::new();
        let mut events = vec![
            make_row(EventType::Focus, 1_748_772_000_000, "org.mozilla.Firefox"),
            make_row(EventType::Unfocus, 1_748_772_050_000, "org.mozilla.Firefox"),
            make_row(EventType::Focus, 1_748_772_100_000, "com.Code.App"),
        ];
        let tl = build_day_timeline(&mut events, date, &names, test_now(), None);
        // Firefox block + gap + Code open block (gap fills between 5s and 10s)
        assert_eq!(tl.blocks.len(), 3);
        assert!(tl.blocks[0].end.is_some());
        assert!(tl.blocks[1].is_gap);
        let last = &tl.blocks[2];
        assert_eq!(last.app_class, "com.Code.App");
        assert!(last.end.is_none());
    }

    #[test]
    fn test_gap_filling_between_blocks() {
        let date = NaiveDate::from_ymd_opt(2025, 6, 1).unwrap();
        let names = HashMap::new();
        // Firefox from 10:00:00 to 10:00:10, Code from 10:00:30 to 10:00:40
        let mut events = vec![
            make_row(EventType::Focus, 1_748_772_000_000, "org.mozilla.Firefox"),
            make_row(EventType::Unfocus, 1_748_772_010_000, "org.mozilla.Firefox"),
            make_row(EventType::Focus, 1_748_772_030_000, "com.Code.App"),
            make_row(EventType::Unfocus, 1_748_772_040_000, "com.Code.App"),
        ];
        let tl = build_day_timeline(&mut events, date, &names, test_now(), None);
        assert_eq!(tl.blocks.len(), 3); // block + gap + block
        assert!(!tl.blocks[0].is_gap);
        assert!(tl.blocks[1].is_gap);
        assert!(!tl.blocks[2].is_gap);
        let gap = &tl.blocks[1];
        assert_eq!(gap.app_class, "");
        assert_eq!(gap.display_name, "");
    }

    #[test]
    fn test_idle_closes_focus_interval() {
        let date = NaiveDate::from_ymd_opt(2025, 6, 1).unwrap();
        let names = HashMap::new();
        let mut events = vec![
            make_row(EventType::Focus, 1_748_772_000_000, "org.mozilla.Firefox"),
            make_row(EventType::Idle, 1_748_772_010_000, "org.mozilla.Firefox"),
            make_row(EventType::Focus, 1_748_772_030_000, "com.Code.App"),
            make_row(EventType::Unfocus, 1_748_772_040_000, "com.Code.App"),
        ];
        let tl = build_day_timeline(&mut events, date, &names, test_now(), None);
        assert_eq!(tl.blocks.len(), 3); // block + gap + block
        assert!(!tl.blocks[0].is_gap);
        assert_eq!(tl.blocks[0].app_class, "org.mozilla.Firefox");
        assert!(tl.blocks[1].is_gap);
        assert!(!tl.blocks[2].is_gap);
        assert_eq!(tl.blocks[2].app_class, "com.Code.App");
        assert_eq!(tl.total_focus_millis, 20_000);
    }

    #[test]
    fn test_close_without_open_or_seed_emits_nothing() {
        let date = NaiveDate::from_ymd_opt(2025, 6, 2).unwrap();
        let names = HashMap::new();
        let mut events = vec![make_row(
            EventType::Unfocus,
            1_748_822_400_000 + 600_000,
            "org.mozilla.Firefox",
        )];
        let tl = build_day_timeline(&mut events, date, &names, test_now(), None);
        assert!(tl.blocks.is_empty());
        assert_eq!(tl.total_focus_millis, 0);
    }

    #[test]
    fn test_stale_seed_clipped_to_day_start() {
        let date = NaiveDate::from_ymd_opt(2025, 6, 2).unwrap();
        let day_start = 1_748_822_400_000i64;
        let names = HashMap::new();
        let mut events = vec![make_row(
            EventType::Unfocus,
            day_start + 600_000,
            "org.mozilla.Firefox",
        )];
        let stale_ts = day_start - 2 * 86_400_000 - 3_600_000;
        let seed = Some((stale_ts, AppClass::new("org.mozilla.Firefox").unwrap()));
        let tl = build_day_timeline(&mut events, date, &names, test_now(), seed);
        assert_eq!(tl.blocks.len(), 1);
        assert!(!tl.blocks[0].is_gap);
        assert_eq!(tl.blocks[0].app_class, "org.mozilla.Firefox");
        assert_eq!(tl.blocks[0].start.timestamp_millis(), day_start);
        assert_eq!(
            tl.blocks[0].end.unwrap().timestamp_millis(),
            day_start + 600_000
        );
        assert_eq!(tl.total_focus_millis, 600_000);
    }

    #[test]
    fn test_seed_chains_into_first_in_range_focus() {
        let date = NaiveDate::from_ymd_opt(2025, 6, 2).unwrap();
        let day_start = 1_748_822_400_000i64;
        let names = HashMap::new();
        let mut events = vec![
            make_row(EventType::Focus, day_start + 1_800_000, "com.Code.App"),
            make_row(EventType::Unfocus, day_start + 2_400_000, "com.Code.App"),
        ];
        let seed = Some((
            day_start - 3_600_000,
            AppClass::new("org.mozilla.Firefox").unwrap(),
        ));
        let tl = build_day_timeline(&mut events, date, &names, test_now(), seed);
        assert_eq!(tl.blocks.len(), 2);
        assert_eq!(tl.blocks[0].app_class, "org.mozilla.Firefox");
        assert_eq!(tl.blocks[0].start.timestamp_millis(), day_start);
        assert_eq!(
            tl.blocks[0].end.unwrap().timestamp_millis(),
            day_start + 1_800_000
        );
        assert_eq!(tl.blocks[1].app_class, "com.Code.App");
        assert_eq!(tl.total_focus_millis, 2_400_000);
    }

    #[test]
    fn test_seed_without_events_renders_open_block_from_midnight() {
        let date = NaiveDate::from_ymd_opt(2025, 6, 2).unwrap();
        let day_start = 1_748_822_400_000i64;
        let names = HashMap::new();
        let seed = Some((
            day_start - 3_600_000,
            AppClass::new("org.mozilla.Firefox").unwrap(),
        ));
        let tl = build_day_timeline(&mut [], date, &names, test_now(), seed);
        assert_eq!(tl.blocks.len(), 1);
        assert_eq!(tl.blocks[0].start.timestamp_millis(), day_start);
        assert!(tl.blocks[0].end.is_none());
    }

    #[test]
    fn test_hourly_buckets_split_cross_midnight_block() {
        let date = NaiveDate::from_ymd_opt(2025, 6, 1).unwrap();
        let day_start = 1_748_736_000_000i64;
        let names = HashMap::new();
        let mut events = vec![
            make_row(
                EventType::Focus,
                day_start + 82_800_000,
                "org.mozilla.Firefox",
            ),
            make_row(
                EventType::Unfocus,
                day_start + 86_400_000 + 1_800_000,
                "org.mozilla.Firefox",
            ),
        ];
        let tl = build_day_timeline(&mut events, date, &names, test_now(), None);
        let now = DateTime::from_timestamp_millis(day_start + 86_400_000 + 1_800_000).unwrap();
        let buckets = compute_hourly_buckets(&tl, now);
        assert_eq!(buckets[23].total_millis, 3_600_000);
        let total: i64 = buckets.iter().map(|b| b.total_millis).sum();
        assert_eq!(total, 3_600_000);
    }

    #[test]
    fn test_hourly_buckets_single_block() {
        let date = NaiveDate::from_ymd_opt(2025, 6, 1).unwrap();
        let names = HashMap::new();
        let mut events = vec![
            // 10:00:00 to 10:30:00
            make_row(EventType::Focus, 1_748_772_000_000, "org.mozilla.Firefox"),
            make_row(EventType::Unfocus, 1_748_773_800_000, "org.mozilla.Firefox"),
        ];
        let tl = build_day_timeline(&mut events, date, &names, test_now(), None);
        let now = DateTime::from_timestamp_millis(1_748_773_800_000).unwrap();
        let buckets = compute_hourly_buckets(&tl, now);
        // Block is in hour 10
        assert_eq!(buckets[10].fragments.len(), 1);
        assert!(!buckets[10].fragments[0].is_gap);
        assert_eq!(buckets[10].fragments[0].millis, 1_800_000);
        assert_eq!(buckets[10].total_millis, 1_800_000);
        for (h, bucket) in buckets.iter().enumerate().take(10) {
            assert!(bucket.fragments.is_empty(), "hour {h} should be empty");
        }
        for (h, bucket) in buckets.iter().enumerate().take(24).skip(11) {
            assert!(bucket.fragments.is_empty(), "hour {h} should be empty");
        }
    }

    #[test]
    fn test_hourly_buckets_spanning_hours() {
        let date = NaiveDate::from_ymd_opt(2025, 6, 1).unwrap();
        let names = HashMap::new();
        // 10:45:00 to 11:15:00 — spans hours 10 and 11
        let mut events = vec![
            make_row(EventType::Focus, 1_748_774_700_000, "org.mozilla.Firefox"),
            make_row(EventType::Unfocus, 1_748_776_500_000, "org.mozilla.Firefox"),
        ];
        let tl = build_day_timeline(&mut events, date, &names, test_now(), None);
        let now = DateTime::from_timestamp_millis(1_748_776_500_000).unwrap();
        let buckets = compute_hourly_buckets(&tl, now);
        // 15 min in hour 10, 15 min in hour 11
        assert_eq!(buckets[10].fragments.len(), 1);
        assert_eq!(buckets[10].fragments[0].millis, 900_000); // 15 min
        assert_eq!(buckets[11].fragments.len(), 1);
        assert_eq!(buckets[11].fragments[0].millis, 900_000); // 15 min
    }

    #[test]
    fn test_open_block_uses_now() {
        let date = NaiveDate::from_ymd_opt(2025, 6, 1).unwrap();
        let names = HashMap::new();
        // Firefox at 10:00:00, no close event
        let mut events = vec![make_row(
            EventType::Focus,
            1_748_772_000_000,
            "org.mozilla.Firefox",
        )];
        let tl = build_day_timeline(&mut events, date, &names, test_now(), None);
        // now = 10:30:00
        let now_ms = 1_748_773_800_000;
        let now_dt = DateTime::from_timestamp_millis(now_ms).unwrap();
        let buckets = compute_hourly_buckets(&tl, now_dt);
        assert_eq!(buckets[10].fragments.len(), 1);
        assert_eq!(buckets[10].fragments[0].millis, 1_800_000);
    }
}
