//! Dashboard gpui rendering — all render logic, no D-Bus/data access.
//! Consumes `DashboardViewModel` from `domain.rs` built by `data.rs`.

use chrono::Utc;
use gpui::prelude::*;
use gpui::*;
use gpui_component::{h_flex, v_flex};

use crate::chart::{empty_state, pie_chart_panel};
use crate::components::{card, format_duration, stat_card};
use crate::theme::{self, rad, sp};

use super::data::compute_hourly_buckets;
use super::data::compute_kpis;
use super::domain::{AppListEntry, BlockCardInfo, DashboardViewModel, DayTimeline};

/// Render the complete dashboard view from a ViewModel.
pub fn render_dashboard_view(cx: &App, vm: &DashboardViewModel) -> impl IntoElement {
    let kpis = compute_kpis(vm);

    v_flex()
        .gap_4()
        .child(
            h_flex()
                .gap_4()
                .child(stat_card(
                    cx,
                    &format_duration(kpis.total_millis),
                    "Total Screen Time",
                    Some(theme::primary(cx)),
                ))
                .child(stat_card(
                    cx,
                    &kpis.top_app,
                    &format!("Top App \u{B7} {}", format_duration(kpis.top_app_millis)),
                    Some(theme::secondary(cx)),
                ))
                .child(stat_card(
                    cx,
                    &kpis.active_blocks.to_string(),
                    "Active Blocks",
                    Some(theme::danger(cx)),
                )),
        )
        .child(card(
            cx,
            Some("Day Timeline"),
            vec![
                if let Some(ref tl) = vm.day_timeline {
                    day_timeline_chart(cx, tl)
                } else {
                    empty_state(cx, "No timeline data for this day.").into_any_element()
                }
                .into_any_element(),
            ],
        ))
        .child(
            v_flex()
                .gap_4()
                .child(card(
                    cx,
                    Some("By App"),
                    vec![pie_chart_panel(cx, &vm.pie_app, true).into_any_element()],
                ))
                .child(card(
                    cx,
                    Some("By Category"),
                    vec![pie_chart_panel(cx, &vm.pie_category, true).into_any_element()],
                )),
        )
        .child(card(
            cx,
            Some("Top Apps"),
            vec![app_list_panel(cx, &vm.top_apps).into_any_element()],
        ))
        .when(!vm.block_cards.is_empty(), |el| {
            el.child(card(
                cx,
                Some("Currently Blocked"),
                vm.block_cards
                    .iter()
                    .map(|c| block_card(cx, c).into_any_element())
                    .collect::<Vec<_>>(),
            ))
        })
}

fn app_list_panel(cx: &App, entries: &[AppListEntry]) -> AnyElement {
    if entries.is_empty() {
        return empty_state(cx, "No usage data yet.").into_any_element();
    }

    let rows: Vec<AnyElement> = entries
        .iter()
        .map(|entry| {
            let blocked_color = if entry.is_blocked {
                theme::danger(cx)
            } else {
                theme::success(cx)
            };
            let dot_color = if entry.display_name.is_empty() {
                theme::color_from_str(&entry.app_id)
            } else {
                theme::color_from_str(&entry.display_name)
            };

            h_flex()
                .px(sp::MD)
                .py(sp::SM)
                .rounded(rad::md())
                .hover(|s| s.bg(theme::border(cx)))
                .gap_4()
                .items_center()
                .child(
                    div()
                        .text_xs()
                        .text_color(theme::text_muted(cx))
                        .w(px(28.0))
                        .child(format!("#{}", entry.rank)),
                )
                .child(div().size(px(10.0)).rounded(rad::full()).bg(dot_color))
                .child(
                    div()
                        .text_sm()
                        .flex_1()
                        .text_color(theme::text_primary(cx))
                        .child(entry.display_name.clone()),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(blocked_color)
                        .child(if entry.is_blocked { "BLOCKED" } else { "" }),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(theme::text_secondary(cx))
                        .child(format!("{:.1}%", entry.percentage)),
                )
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::BOLD)
                        .text_color(theme::text_primary(cx))
                        .child(format_duration(entry.total_millis)),
                )
                .into_any_element()
        })
        .collect();

    v_flex().gap_1().children(rows).into_any_element()
}

fn block_card(cx: &App, info: &BlockCardInfo) -> AnyElement {
    let now = Utc::now();
    let duration = now.signed_duration_since(info.blocked_since);
    let ago = if duration.num_minutes() < 1 {
        "just now".to_string()
    } else if duration.num_minutes() < 60 {
        format!("{} minutes ago", duration.num_minutes())
    } else if duration.num_hours() < 24 {
        format!("{} hours ago", duration.num_hours())
    } else {
        format!("{} days ago", duration.num_days())
    };

    let display = if info.display_name.is_empty() {
        &info.app_id
    } else {
        &info.display_name
    };

    h_flex()
        .gap_3()
        .items_center()
        .child(div().size(px(10.0)).rounded(rad::full()).bg(theme::danger(cx)))
        .child(
            v_flex()
                .gap_1()
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::BOLD)
                        .text_color(theme::text_primary(cx))
                        .child(format!("{} \u{2014} Blocked {}", display, ago)),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(theme::text_secondary(cx))
                        .child("Daily limit reached. Switch to the window and use the overlay controls to continue."),
                ),
        )
.into_any_element()
}

/// Create a positioned bar segment for the timeline chart.
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
                    let color = theme::color_from_str(&frag.app_id);
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
