//! Dashboard ViewModel builder and data transformations.
//!
//! All functions are pure — no gpui, no async. Consumed by UI layer.

use std::collections::{BTreeMap, HashMap};

use chrono::{DateTime, NaiveDate, Utc};
use wellbeing_core::{
    AppCategoryRow, Category, DailySummary, DailyUsageEntry, DateRange, DayEventRow,
    event_types::is_close_event_type,
};

use super::domain::{
    AppListEntry, BlockCardInfo, DashboardViewModel, DayTimeline, HourlyBucket, Kpis,
    TimelineBlock, TimelineFragment,
};
use crate::chart::Slice;

/// Transform D-Bus cache data into a `DashboardViewModel`.
pub fn build_dashboard_viewmodel(
    range: DateRange,
    summaries: &[DailySummary],
    categories: &[Category],
    app_categories: &[AppCategoryRow],
    block_cards: Vec<BlockCardInfo>,
    day_timeline: Option<DayTimeline>,
) -> DashboardViewModel {
    let usage: Vec<DailyUsageEntry> = summaries
        .iter()
        .flat_map(|s| s.entries.iter().cloned())
        .collect();

    let pie_app = build_app_slices(&usage, app_categories);
    let pie_category = build_category_slices(&usage, categories, app_categories);
    let top_apps = build_top_apps(&usage, app_categories);

    DashboardViewModel {
        date_range: range,
        pie_app,
        pie_category,
        top_apps,
        block_cards,
        day_timeline,
    }
}

fn build_app_slices(usage: &[DailyUsageEntry], app_categories: &[AppCategoryRow]) -> Vec<Slice> {
    let mut by_app: HashMap<String, f64> = HashMap::new();
    let mut total: f64 = 0.0;
    for entry in usage {
        let minutes = entry.total_millis as f64;
        *by_app.entry(entry.app_id.clone()).or_insert(0.0) += minutes;
        total += minutes;
    }

    if total <= 0.0 {
        return Vec::new();
    }

    let meta: HashMap<&str, &str> = app_categories
        .iter()
        .map(|ac| (ac.app_id.as_str(), ac.display_name.as_str()))
        .collect();

    let mut slices: Vec<Slice> = by_app
        .into_iter()
        .map(|(app_id, app_minutes)| {
            let display_name = meta
                .get(app_id.as_str())
                .copied()
                .unwrap_or(&app_id)
                .to_string();
            Slice {
                percentage: (app_minutes / total) * 100.0,
                app_id,
                display_name,
                color: String::new(),
            }
        })
        .collect();

    slices.sort_by(|a, b| {
        b.percentage
            .partial_cmp(&a.percentage)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    slices
}

fn build_category_slices(
    usage: &[DailyUsageEntry],
    categories: &[Category],
    app_categories: &[AppCategoryRow],
) -> Vec<Slice> {
    let app_to_cat: HashMap<&str, i64> = app_categories
        .iter()
        .map(|ac| (ac.app_id.as_str(), ac.category_id))
        .collect();

    let cat_map: HashMap<i64, &Category> = categories.iter().map(|c| (c.id.0, c)).collect();

    let mut by_cat: HashMap<String, (f64, String)> = HashMap::new();
    let mut total: f64 = 0.0;
    for entry in usage {
        let minutes = entry.total_millis as f64;
        let cat_id = app_to_cat.get(entry.app_id.as_str()).copied().unwrap_or(0);
        let cat_name = cat_map
            .get(&cat_id)
            .map(|c| c.name.clone())
            .unwrap_or_else(|| "Uncategorized".into());
        let entry = by_cat.entry(cat_name).or_insert((0.0, String::new()));
        entry.0 += minutes;
        total += minutes;
    }

    if total <= 0.0 {
        return Vec::new();
    }

    let mut slices: Vec<Slice> = by_cat
        .into_iter()
        .map(|(name, (cat_minutes, color))| Slice {
            percentage: (cat_minutes / total) * 100.0,
            app_id: name.clone(),
            display_name: name,
            color,
        })
        .collect();

    slices.sort_by(|a, b| {
        b.percentage
            .partial_cmp(&a.percentage)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    slices
}

fn build_top_apps(
    usage: &[DailyUsageEntry],
    app_categories: &[AppCategoryRow],
) -> Vec<AppListEntry> {
    let app_meta: HashMap<&str, (&str, Option<String>)> = app_categories
        .iter()
        .map(|ac| {
            let color = if ac.category_id > 0 {
                Some(format!("cat_{}", ac.category_id))
            } else {
                None
            };
            (ac.app_id.as_str(), (ac.display_name.as_str(), color))
        })
        .collect();

    let mut by_app: BTreeMap<String, i64> = BTreeMap::new();
    let mut grand_total: f64 = 0.0;
    for entry in usage {
        *by_app.entry(entry.app_id.clone()).or_insert(0) += entry.total_millis;
        grand_total += entry.total_millis as f64;
    }

    if grand_total <= 0.0 {
        return Vec::new();
    }

    let mut entries: Vec<AppListEntry> = by_app
        .into_iter()
        .map(|(app_id, total_millis)| {
            let (display_name, category_color) = app_meta
                .get(app_id.as_str())
                .map(|(name, color)| (name.to_string(), color.clone()))
                .unwrap_or_else(|| (app_id.clone(), None));
            AppListEntry {
                rank: 0,
                total_millis,
                percentage: (total_millis as f64 / grand_total) * 100.0,
                app_id,
                display_name,
                category_color,
                is_blocked: false,
            }
        })
        .collect();

    entries.sort_by_key(|a| std::cmp::Reverse(a.total_millis));
    for (i, entry) in entries.iter_mut().enumerate() {
        entry.rank = i + 1;
    }

    entries.truncate(10);
    entries
}

// ── Helpers ────────────────────────────────────────────────────────────────

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

/// Fill time gaps between consecutive blocks by inserting synthetic gap blocks.
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
    events: Vec<DayEventRow>,
    date: NaiveDate,
    app_names: &HashMap<String, String>,
) -> DayTimeline {
    let mut events = events;
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

/// Compute the KPI summary for the stat row.
pub fn compute_kpis(vm: &DashboardViewModel) -> Kpis {
    let total_millis: i64 = vm.top_apps.iter().map(|a| a.total_millis).sum();
    let top = vm.top_apps.first();
    Kpis {
        total_millis,
        top_app: top
            .map(|t| t.display_name.clone())
            .unwrap_or_else(|| "\u{2014}".into()),
        top_app_millis: top.map(|t| t.total_millis).unwrap_or(0),
        active_blocks: vm.block_cards.len(),
    }
}
