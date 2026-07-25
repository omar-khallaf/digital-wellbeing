//! Day timeline and hourly bucket computations.
//!
//! Converts raw D-Bus event rows into focus interval blocks and hourly
//! breakdowns for the timeline view. Pure functions — no gpui, no async.

use std::collections::HashMap;

use chrono::{DateTime, NaiveDate, Utc};
use wellbeing_core::{DayEventRow, event_types::is_close_event_type};

use super::domain::{DayTimeline, HourlyBucket, TimelineBlock, TimelineFragment};

fn ms_to_dt(ts: i64) -> DateTime<Utc> {
    DateTime::from_timestamp_millis(ts)
        .unwrap_or_else(|| panic!("valid epoch millis in event row, got {ts}"))
}

fn display_name(app_id: &str, names: &HashMap<String, String>) -> String {
    names
        .get(app_id)
        .cloned()
        .unwrap_or_else(|| app_id.to_string())
}

fn make_focus_block(
    start_ts: i64,
    end_ts: i64,
    app_id: &str,
    names: &HashMap<String, String>,
    event_type: u8,
) -> TimelineBlock {
    TimelineBlock {
        app_id: app_id.to_string(),
        display_name: display_name(app_id, names),
        start: ms_to_dt(start_ts),
        end: Some(ms_to_dt(end_ts)),
        event_type,
        is_gap: false,
    }
}

/// Insert gap blocks wherever consecutive focus blocks are not adjacent.
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
                    app_id: String::new(),
                    display_name: String::new(),
                    start: prev_end,
                    end: Some(next_start),
                    event_type: 255,
                    is_gap: true,
                };
                blocks.insert(i + 1, gap);
                i += 1; // skip past the newly inserted gap
            }
        }
        i += 1;
    }
}

/// Build a `DayTimeline` from raw D-Bus event rows for a single day.
///
/// Only WindowFocused(0) and termination events (CLOSE_EVENT_TYPES: 1,4,5,6,7)
/// are used to build focus intervals. Idle(2), Resumed(3), and other event types
/// are ignored — time gaps between focus blocks are filled by `fill_time_gaps`.
///
/// **Algorithm**
/// 1. Sort events by timestamp ASC.
/// 2. Walk events: WindowFocused(0) opens a new interval (closing any previous
///    unmatched focus). Termination events close the current open interval.
/// 3. Unmatched focus at end of day → `end: None`.
/// 4. Fill remaining time gaps between consecutive blocks.
pub fn build_day_timeline(
    mut events: Vec<DayEventRow>,
    date: NaiveDate,
    app_names: &HashMap<String, String>,
) -> DayTimeline {
    events.sort_by_key(|e| e.timestamp);

    let mut blocks: Vec<TimelineBlock> = Vec::new();
    let mut pending_focus: Option<(i64, String)> = None;

    for event in &events {
        match event.event_type {
            0 => {
                if let Some((st, ref aid)) = pending_focus.take() {
                    blocks.push(make_focus_block(st, event.timestamp, aid, app_names, 0));
                }
                pending_focus = Some((event.timestamp, event.app_id.clone()));
            }
            e if is_close_event_type(e) => {
                if let Some((st, ref aid)) = pending_focus.take() {
                    blocks.push(make_focus_block(st, event.timestamp, aid, app_names, e));
                }
            }
            _ => {}
        }
    }

    if let Some((st, ref aid)) = pending_focus.take() {
        blocks.push(TimelineBlock {
            app_id: aid.clone(),
            display_name: display_name(aid, app_names),
            start: ms_to_dt(st),
            end: None,
            event_type: 0,
            is_gap: false,
        });
    }

    blocks.sort_by_key(|b| b.start);

    fill_time_gaps(&mut blocks);

    let total_focus_millis: i64 = blocks
        .iter()
        .filter(|b| !b.is_gap)
        .filter_map(|b| {
            b.end
                .map(|e| e.timestamp_millis() - b.start.timestamp_millis())
        })
        .sum();

    DayTimeline {
        date,
        blocks,
        total_focus_millis,
    }
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

    // Precompute start-of-hour timestamps to avoid repeated and_hms_opt calls
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
            let overlap_start = start_ms.max(hour_start);
            let overlap_end = end_ms.min(hour_end);
            let millis = (overlap_end - overlap_start).max(0);

            if millis == 0 {
                continue;
            }

            let start_offset = overlap_start - actual_hour_start;
            buckets[h].fragments.push(TimelineFragment {
                app_id: block.app_id.clone(),
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

    fn make_row(event_type: u8, timestamp: i64, app_id: &str) -> DayEventRow {
        DayEventRow {
            id: 0,
            event_type,
            timestamp,
            app_id: app_id.to_string(),
            title: String::new(),
            user_id: 0,
        }
    }

    #[test]
    fn test_empty_events_returns_empty_timeline() {
        let date = NaiveDate::from_ymd_opt(2025, 6, 1).unwrap();
        let names = HashMap::new();
        let tl = build_day_timeline(vec![], date, &names);
        assert!(tl.blocks.is_empty());
        assert_eq!(tl.total_focus_millis, 0);
        assert_eq!(tl.date, date);
    }

    #[test]
    fn test_single_focus_with_close() {
        let date = NaiveDate::from_ymd_opt(2025, 6, 1).unwrap();
        let names = HashMap::new();
        let events = vec![
            make_row(0, 1_748_772_000_000, "org.mozilla.Firefox"),
            make_row(1, 1_748_772_050_000, "org.mozilla.Firefox"),
        ];
        let tl = build_day_timeline(events, date, &names);
        assert_eq!(tl.blocks.len(), 1);
        assert!(!tl.blocks[0].is_gap);
        assert_eq!(tl.blocks[0].app_id, "org.mozilla.Firefox");
        assert!(tl.blocks[0].end.is_some());
        assert_eq!(tl.total_focus_millis, 50_000);
    }

    #[test]
    fn test_unmatched_focus_at_end() {
        let date = NaiveDate::from_ymd_opt(2025, 6, 1).unwrap();
        let names = HashMap::new();
        let events = vec![
            make_row(0, 1_748_772_000_000, "org.mozilla.Firefox"),
            make_row(1, 1_748_772_050_000, "org.mozilla.Firefox"),
            make_row(0, 1_748_772_100_000, "com.Code.App"),
        ];
        let tl = build_day_timeline(events, date, &names);
        // Firefox block + gap + Code open block (gap fills between 5s and 10s)
        assert_eq!(tl.blocks.len(), 3);
        assert!(tl.blocks[0].end.is_some());
        assert!(tl.blocks[1].is_gap);
        let last = &tl.blocks[2];
        assert_eq!(last.app_id, "com.Code.App");
        assert!(last.end.is_none());
    }

    #[test]
    fn test_gap_filling_between_blocks() {
        let date = NaiveDate::from_ymd_opt(2025, 6, 1).unwrap();
        let names = HashMap::new();
        // Firefox from 10:00:00 to 10:00:10, Code from 10:00:30 to 10:00:40
        let events = vec![
            make_row(0, 1_748_772_000_000, "org.mozilla.Firefox"),
            make_row(1, 1_748_772_010_000, "org.mozilla.Firefox"),
            make_row(0, 1_748_772_030_000, "com.Code.App"),
            make_row(1, 1_748_772_040_000, "com.Code.App"),
        ];
        let tl = build_day_timeline(events, date, &names);
        assert_eq!(tl.blocks.len(), 3); // block + gap + block
        assert!(!tl.blocks[0].is_gap);
        assert!(tl.blocks[1].is_gap);
        assert!(!tl.blocks[2].is_gap);
        let gap = &tl.blocks[1];
        assert_eq!(gap.app_id, "");
        assert_eq!(gap.display_name, "");
    }

    #[test]
    fn test_idle_events_ignored() {
        let date = NaiveDate::from_ymd_opt(2025, 6, 1).unwrap();
        let names = HashMap::new();
        let events = vec![
            make_row(0, 1_748_772_000_000, "org.mozilla.Firefox"),
            make_row(2, 1_748_772_010_000, ""), // idle — ignored
            make_row(1, 1_748_772_020_000, "org.mozilla.Firefox"),
        ];
        let tl = build_day_timeline(events, date, &names);
        assert_eq!(tl.blocks.len(), 1);
        assert_eq!(tl.total_focus_millis, 20_000);
    }

    #[test]
    fn test_hourly_buckets_single_block() {
        let date = NaiveDate::from_ymd_opt(2025, 6, 1).unwrap();
        let names = HashMap::new();
        let events = vec![
            // 10:00:00 to 10:30:00
            make_row(0, 1_748_772_000_000, "org.mozilla.Firefox"),
            make_row(1, 1_748_773_800_000, "org.mozilla.Firefox"),
        ];
        let tl = build_day_timeline(events, date, &names);
        let now = DateTime::from_timestamp_millis(1_748_773_800_000).unwrap();
        let buckets = compute_hourly_buckets(&tl, now);
        // Block is in hour 10
        assert_eq!(buckets[10].fragments.len(), 1);
        assert!(!buckets[10].fragments[0].is_gap);
        assert_eq!(buckets[10].fragments[0].millis, 1_800_000);
        assert_eq!(buckets[10].total_millis, 1_800_000);
        // All other hours should be empty
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
        let events = vec![
            make_row(0, 1_748_774_700_000, "org.mozilla.Firefox"),
            make_row(1, 1_748_776_500_000, "org.mozilla.Firefox"),
        ];
        let tl = build_day_timeline(events, date, &names);
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
        let events = vec![make_row(0, 1_748_772_000_000, "org.mozilla.Firefox")];
        let tl = build_day_timeline(events, date, &names);
        // now = 10:30:00
        let now_ms = 1_748_773_800_000;
        let now_dt = DateTime::from_timestamp_millis(now_ms).unwrap();
        let buckets = compute_hourly_buckets(&tl, now_dt);
        assert_eq!(buckets[10].fragments.len(), 1);
        // should be 30 min (1800s) from 10:00 to 10:30
        assert_eq!(buckets[10].fragments[0].millis, 1_800_000);
    }
}
