//! Timeline chart rendering — 24-hour horizontal bar with per-app segments.

use chrono::Utc;
use gpui::prelude::*;
use gpui::*;
use gpui_component::v_flex;

use crate::chart::empty_state;
use crate::theme;

use crate::dashboard::domain::DayTimeline;
use crate::dashboard::timeline::compute_hourly_buckets;

/// All segments share the same absolute positioning within the track.
fn timeline_bar_segment(left: f32, width: f32, color: Hsla, track_height: Pixels) -> AnyElement {
    div()
        .absolute()
        .top(px(0.0))
        .h(track_height)
        .left(DefiniteLength::Fraction(left))
        .w(DefiniteLength::Fraction(width))
        .bg(color)
        .into_any_element()
}

/// Render a 24-hour horizontal timeline bar with hourly interval dividers and
/// multi-colored per-app segments within each hour.
pub fn day_timeline_chart(cx: &App, timeline: &DayTimeline) -> AnyElement {
    let buckets = compute_hourly_buckets(timeline, Utc::now());
    if buckets.iter().all(|b| b.fragments.is_empty()) {
        return empty_state(cx, "No timeline data for this day.").into_any_element();
    }

    let track_height = px(48.0);
    let border = theme::border(cx);
    let muted = theme::text_muted(cx);

    // Each fragment (focus or gap) is positioned at its absolute start offset
    // within the hour — no cumulative stacking, so idle gaps between intervals
    // in the same hour are preserved visually.
    let fragments: Vec<AnyElement> = buckets
        .iter()
        .enumerate()
        .flat_map(|(hour, bucket)| {
            let total = 3_600_000i64;
            let hour_fraction = 1.0 / 24.0;
            let mut els: Vec<AnyElement> = Vec::new();

            if bucket.fragments.is_empty() {
                els.push(timeline_bar_segment(
                    hour as f32 / 24.0,
                    hour_fraction,
                    border,
                    track_height,
                ));
                return els;
            }

            // Idle gap before the first fragment if it doesn't start at
            // the hour boundary (e.g. tracking began at 05:36, not 05:00).
            if bucket.fragments[0].start_offset > 0 {
                let idle_ratio = bucket.fragments[0].start_offset as f32 / total as f32;
                els.push(timeline_bar_segment(
                    hour as f32 / 24.0,
                    idle_ratio * hour_fraction,
                    border,
                    track_height,
                ));
            }

            for frag in &bucket.fragments {
                let seg_ratio = frag.millis as f32 / total as f32;
                let left = (hour as f32 + frag.start_offset as f32 / total as f32) / 24.0;
                let width = seg_ratio * hour_fraction;

                if frag.is_gap {
                    els.push(timeline_bar_segment(left, width, border, track_height));
                } else {
                    let color = theme::color_from_str(&frag.app_class);
                    // Slightly transparent so the 1 px divider lines show through.
                    let segment_bg = Hsla { a: 0.75, ..color };
                    els.push(timeline_bar_segment(left, width, segment_bg, track_height));
                }
            }

            let last = bucket.fragments.last().unwrap();
            let last_end_offset = last.start_offset + last.millis;
            if last_end_offset < total {
                let left = (hour as f32 + last_end_offset as f32 / total as f32) / 24.0;
                let width = (total - last_end_offset) as f32 / total as f32 * hour_fraction;
                els.push(timeline_bar_segment(left, width, border, track_height));
            }

            els
        })
        .collect();

    // Vertical divider lines between every hour (painted on top of segments).
    let divider_opacity = 0.45;
    let dividers: Vec<AnyElement> = (1..24)
        .map(|hour| {
            div()
                .absolute()
                .top(px(0.0))
                .h(track_height)
                .left(DefiniteLength::Fraction(hour as f32 / 24.0))
                .w(px(1.0))
                .bg(Hsla {
                    a: divider_opacity,
                    ..muted
                })
                .into_any_element()
        })
        .collect();

    // Each label lives in a 1/24‑wide wrapper centred on the divider so the
    // text is visually aligned below its line rather than starting at it.
    let hour_markers: Vec<AnyElement> = (1..24)
        .map(|hour| {
            div()
                .absolute()
                .top(px(0.0))
                .left(DefiniteLength::Fraction((hour as f32 - 0.5) / 24.0))
                .w(DefiniteLength::Fraction(1.0 / 24.0))
                .h_full()
                .flex()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .text_size(px(9.0))
                        .text_color(Hsla { a: 0.75, ..muted })
                        .child(format!("{:02}:00", hour)),
                )
                .into_any_element()
        })
        .collect();

    v_flex()
        .gap_1()
        .child(
            div()
                .relative()
                .w_full()
                .h(track_height)
                .bg(border)
                .rounded(px(4.0))
                .overflow_hidden()
                .children(fragments)
                .children(dividers),
        )
        .child(div().relative().w_full().h(px(40.0)).children(hour_markers))
        .into_any_element()
}
