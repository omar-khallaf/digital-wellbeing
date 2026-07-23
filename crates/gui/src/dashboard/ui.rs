//! Dashboard gpui rendering — all render logic, no D-Bus/data access.
//! Consumes `DashboardViewModel` from `domain.rs` built by `data.rs`.

use chrono::Utc;
use gpui::prelude::*;
use gpui::*;
use gpui_component::{h_flex, v_flex};

use crate::chart::{daily_bar_chart, empty_state, pie_chart_panel};
use crate::components::{card, format_duration, stat_card};
use crate::theme::{self, rad, sp};

use super::data::compute_kpis;
use super::domain::{AppListEntry, BlockCardInfo, DashboardViewModel};

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
            Some("Daily Screen Time"),
            vec![daily_bar_chart(cx, &vm.bar_chart).into_any_element()],
        ))
        .child(
            h_flex()
                .gap_4()
                .child(div().flex_1().child(card(
                    cx,
                    Some("By App"),
                    vec![pie_chart_panel(cx, &vm.pie_app, true).into_any_element()],
                )))
                .child(div().flex_1().child(card(
                    cx,
                    Some("By Category"),
                    vec![pie_chart_panel(cx, &vm.pie_category, true).into_any_element()],
                ))),
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
            let cat_color = entry
                .category_color
                .as_deref()
                .and_then(|c| theme::parse_hex(c.trim_start_matches("cat_")))
                .unwrap_or_else(|| theme::text_muted(cx));

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
                .child(div().size(px(10.0)).rounded(rad::full()).bg(cat_color))
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
