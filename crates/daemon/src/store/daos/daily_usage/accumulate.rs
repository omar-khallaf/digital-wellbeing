//! Standalone accumulation helpers that aggregate closed/open interval
//! durations into the per-app, per-title, and per-category maps.

use std::collections::HashMap;

use wellbeing_core::Uid;

use super::deltas::{AggKeyApp, AggKeyTitle, OpenFocus};

/// Split `[prev.ts_ms, close_ts_ms]` across UTC day boundaries, compute
/// per-date durations, and accumulate into the closed aggregates.
pub(crate) fn accumulate_closed(
    prev: &OpenFocus,
    close_ts_ms: i64,
    uid: Uid,
    agg: &mut HashMap<(Uid, AggKeyApp), i64>,
    agg_title: &mut HashMap<(Uid, AggKeyTitle), i64>,
) {
    let day_ms: i64 = 86_400_000;
    let start_of_day_prev = prev.ts_ms - (prev.ts_ms % day_ms);
    let start_of_day_close = close_ts_ms - (close_ts_ms % day_ms);

    if start_of_day_prev == start_of_day_close {
        let date = daily_date_str(prev.ts_ms);
        let dur = close_ts_ms - prev.ts_ms;
        *agg.entry((
            uid,
            AggKeyApp {
                date: date.clone(),
                app_id: prev.app_id,
            },
        ))
        .or_insert(0) += dur;
        *agg_title
            .entry((
                uid,
                AggKeyTitle {
                    date,
                    app_class: prev.app_class.to_string(),
                    title: prev.title.clone(),
                },
            ))
            .or_insert(0) += dur;
    } else {
        let next_midnight = start_of_day_prev + day_ms;
        let first_dur = next_midnight - prev.ts_ms;
        let second_dur = close_ts_ms - next_midnight;

        let date1 = daily_date_str(prev.ts_ms);
        *agg.entry((
            uid,
            AggKeyApp {
                date: date1.clone(),
                app_id: prev.app_id,
            },
        ))
        .or_insert(0) += first_dur;
        *agg_title
            .entry((
                uid,
                AggKeyTitle {
                    date: date1,
                    app_class: prev.app_class.to_string(),
                    title: prev.title.clone(),
                },
            ))
            .or_insert(0) += first_dur;

        let date2 = daily_date_str(next_midnight);
        *agg.entry((
            uid,
            AggKeyApp {
                date: date2.clone(),
                app_id: prev.app_id,
            },
        ))
        .or_insert(0) += second_dur;
        *agg_title
            .entry((
                uid,
                AggKeyTitle {
                    date: date2,
                    app_class: prev.app_class.to_string(),
                    title: prev.title.clone(),
                },
            ))
            .or_insert(0) += second_dur;
    }
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
    let day_ms: i64 = 86_400_000;
    let start_of_day_start = focus.ts_ms - (focus.ts_ms % day_ms);
    let start_of_day_now = now_ms - (now_ms % day_ms);

    if start_of_day_start == start_of_day_now {
        let date = daily_date_str(focus.ts_ms);
        let dur = now_ms - focus.ts_ms;
        *agg.entry((
            uid,
            AggKeyApp {
                date: date.clone(),
                app_id: focus.app_id,
            },
        ))
        .or_insert(0) += dur;
        *agg_title
            .entry((
                uid,
                AggKeyTitle {
                    date,
                    app_class: focus.app_class.to_string(),
                    title: focus.title.clone(),
                },
            ))
            .or_insert(0) += dur;
    } else {
        let next_midnight = start_of_day_start + day_ms;
        let first_dur = next_midnight - focus.ts_ms;
        let second_dur = now_ms - next_midnight;

        let date1 = daily_date_str(focus.ts_ms);
        *agg.entry((
            uid,
            AggKeyApp {
                date: date1.clone(),
                app_id: focus.app_id,
            },
        ))
        .or_insert(0) += first_dur;
        *agg_title
            .entry((
                uid,
                AggKeyTitle {
                    date: date1,
                    app_class: focus.app_class.to_string(),
                    title: focus.title.clone(),
                },
            ))
            .or_insert(0) += first_dur;

        let date2 = daily_date_str(next_midnight);
        *agg.entry((
            uid,
            AggKeyApp {
                date: date2.clone(),
                app_id: focus.app_id,
            },
        ))
        .or_insert(0) += second_dur;
        *agg_title
            .entry((
                uid,
                AggKeyTitle {
                    date: date2,
                    app_class: focus.app_class.to_string(),
                    title: focus.title.clone(),
                },
            ))
            .or_insert(0) += second_dur;
    }
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
}
