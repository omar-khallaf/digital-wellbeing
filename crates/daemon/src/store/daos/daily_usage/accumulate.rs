//! Standalone accumulation helpers that aggregate closed/open interval
//! durations into the per-app, per-title, and per-category maps.

use std::collections::HashMap;

use wellbeing_core::clock::split_by_utc_day;
use wellbeing_core::{AppClass, Uid, WindowTitle};

use super::deltas::{AggKeyApp, AggKeyTitle, OpenFocus};

struct IntervalTarget<'a> {
    app_id: i32,
    app_class: &'a AppClass,
    title: &'a WindowTitle,
    start_ms: i64,
    end_ms: i64,
    uid: Uid,
}

fn accumulate_range(
    target: IntervalTarget<'_>,
    agg: &mut HashMap<(Uid, AggKeyApp), i64>,
    agg_title: &mut HashMap<(Uid, AggKeyTitle), i64>,
) {
    for (day_start_ms, dur) in split_by_utc_day(target.start_ms, target.end_ms) {
        let date = daily_date_str(day_start_ms);
        *agg.entry((
            target.uid,
            AggKeyApp {
                date: date.clone(),
                app_id: target.app_id,
            },
        ))
        .or_insert(0) += dur;
        *agg_title
            .entry((
                target.uid,
                AggKeyTitle {
                    date,
                    app_class: target.app_class.to_string(),
                    app_id: target.app_id,
                    title: target.title.clone(),
                },
            ))
            .or_insert(0) += dur;
    }
}

/// Split `[prev.ts_ms, close_ts_ms]` across UTC day boundaries, compute
/// per-date durations, and accumulate into the closed aggregates.
pub(crate) fn accumulate_closed(
    prev: &OpenFocus,
    close_ts_ms: i64,
    uid: Uid,
    agg: &mut HashMap<(Uid, AggKeyApp), i64>,
    agg_title: &mut HashMap<(Uid, AggKeyTitle), i64>,
) {
    accumulate_range(
        IntervalTarget {
            app_id: prev.app_id,
            app_class: &prev.app_class,
            title: &prev.title,
            start_ms: prev.ts_ms,
            end_ms: close_ts_ms,
            uid,
        },
        agg,
        agg_title,
    );
}

/// Accumulate an open interval (no close event yet) up to `now_ms`, split at
/// midnight if needed.
pub(crate) fn accumulate_open(
    focus: &OpenFocus,
    now_ms: i64,
    uid: Uid,
    agg: &mut HashMap<(Uid, AggKeyApp), i64>,
    agg_title: &mut HashMap<(Uid, AggKeyTitle), i64>,
) {
    accumulate_range(
        IntervalTarget {
            app_id: focus.app_id,
            app_class: &focus.app_class,
            title: &focus.title,
            start_ms: focus.ts_ms,
            end_ms: now_ms,
            uid,
        },
        agg,
        agg_title,
    );
}

/// Convert a unix-epoch-millis timestamp to a `YYYY-MM-DD` date string by
/// interpreting it as a UTC date. This avoids chrono dependency in the hot
/// path when we only need day-partition alignment.
fn daily_date_str(ts_ms: i64) -> String {
    // Compute days since epoch in UTC (avoid chrono crate on hot path).
    let secs = ts_ms / 1000;
    let days = secs / 86_400; // floor division since secs >= 0

    // Civil date from days since epoch (Rata Die algorithm).
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };

    format!("{y:04}-{m:02}-{d:02}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use wellbeing_core::{AppClass, WindowTitle};

    fn test_focus(ts_ms: i64) -> OpenFocus {
        OpenFocus {
            ts_ms,
            app_class: AppClass::new("test.App").unwrap(),
            app_id: 7,
            title: WindowTitle::new("win"),
        }
    }

    fn app_total(agg: &HashMap<(Uid, AggKeyApp), i64>) -> i64 {
        agg.values().sum()
    }

    #[test]
    fn test_daily_date_str_known() {
        // 2026-07-26 00:00:00 UTC in millis
        let ts = 1_785_024_000_000i64;
        assert_eq!(daily_date_str(ts), "2026-07-26");
    }

    #[test]
    fn test_daily_date_str_mid_afternoon() {
        // 2026-07-26 14:30:00 UTC
        let ts = 1_785_076_200_000i64;
        assert_eq!(daily_date_str(ts), "2026-07-26");
    }

    #[test]
    fn test_daily_date_str_epoch() {
        assert_eq!(daily_date_str(0), "1970-01-01");
    }

    #[test]
    fn closed_three_day_span_splits_into_three_rows() {
        let day_ms = 86_400_000i64;
        let start = 2 * day_ms + 1_000;
        let end = start + 2 * day_ms + 4_000;
        let prev = test_focus(start);
        let uid = Uid(1);
        let mut agg = HashMap::new();
        let mut agg_title = HashMap::new();
        accumulate_closed(&prev, end, uid, &mut agg, &mut agg_title);
        assert_eq!(agg.len(), 3);
        assert_eq!(app_total(&agg), end - start);
        let mut vals: Vec<i64> = agg.values().copied().collect();
        vals.sort_unstable();
        assert_eq!(vals[2], day_ms);
        assert_eq!(agg_title.len(), 3);
    }

    #[test]
    fn open_three_day_span_splits_into_three_rows() {
        let day_ms = 86_400_000i64;
        let start = 2 * day_ms + 1_000;
        let now = start + 2 * day_ms + 4_000;
        let focus = test_focus(start);
        let uid = Uid(1);
        let mut agg = HashMap::new();
        let mut agg_title = HashMap::new();
        accumulate_open(&focus, now, uid, &mut agg, &mut agg_title);
        assert_eq!(agg.len(), 3);
        assert_eq!(app_total(&agg), now - start);
        let mut vals: Vec<i64> = agg.values().copied().collect();
        vals.sort_unstable();
        assert_eq!(vals[2], day_ms);
        assert_eq!(agg_title.len(), 3);
    }

    #[test]
    fn single_day_spans_unchanged() {
        let start = 5 * 86_400_000i64 + 1_000;
        let end = start + 9_000;
        let uid = Uid(1);
        let mut agg = HashMap::new();
        let mut agg_title = HashMap::new();
        accumulate_closed(&test_focus(start), end, uid, &mut agg, &mut agg_title);
        assert_eq!(agg.len(), 1);
        assert_eq!(app_total(&agg), 9_000);
        let mut agg2 = HashMap::new();
        let mut agg_title2 = HashMap::new();
        accumulate_open(&test_focus(start), end, uid, &mut agg2, &mut agg_title2);
        assert_eq!(agg2.len(), 1);
        assert_eq!(app_total(&agg2), 9_000);
    }
}
