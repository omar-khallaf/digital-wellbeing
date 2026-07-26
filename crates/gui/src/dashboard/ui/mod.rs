//! Dashboard gpui rendering — all render logic, no D-Bus/data access.
//! Consumes `DashboardViewModel` from `domain.rs` built by `data.rs`.

mod charts;
mod panels;

pub use self::charts::*;
// Internal helpers are pub(super) in sub-modules and imported here,
// not re-exported (they were private in the original single-file layout).
use self::panels::block_card;

use gpui::prelude::*;
use gpui::px;
use gpui::*;
use gpui_component::scroll::ScrollableElement;
use gpui_component::{h_flex, v_flex};

use crate::chart::{empty_state, pie_chart_panel};
use crate::components::{
    self as cmp, AppBadge, AppEntryView, TitleEntryView, card, format_duration, stat_card,
};
use crate::theme;

use super::domain::DashboardViewModel;
use super::viewmodel::compute_kpis;

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
            vec![if let Some(tl) = &vm.day_timeline {
                day_timeline_chart(cx, tl)
            } else {
                empty_state(cx, "No timeline data for this day.")
            }],
        ))
        .child(
            v_flex()
                .gap_4()
                .child(card(
                    cx,
                    Some("By App"),
                    vec![pie_chart_panel(cx, &vm.pie_app, true)],
                ))
                .child(card(
                    cx,
                    Some("By Category"),
                    vec![pie_chart_panel(cx, &vm.pie_category, true)],
                )),
        )
        .child({
            let entries: Vec<AppEntryView> = vm
                .top_apps
                .iter()
                .map(|e| {
                    let dot_color = e
                        .category_color
                        .as_deref()
                        .and_then(theme::parse_hex)
                        .unwrap_or_else(|| {
                            if e.display_name.is_empty() {
                                theme::color_from_str(&e.app_class)
                            } else {
                                theme::color_from_str(&e.display_name)
                            }
                        });
                    let badge = if e.is_blocked {
                        Some(AppBadge {
                            text: "BLOCKED".into(),
                            color: theme::danger(cx),
                        })
                    } else {
                        None
                    };
                    AppEntryView {
                        rank: e.rank,
                        display_name: e.display_name.clone(),
                        total_millis: e.total_millis,
                        percentage: e.percentage,
                        dot_color: Some(dot_color),
                        badge,
                    }
                })
                .collect();
            card(
                cx,
                Some("Top Apps"),
                vec![
                    div()
                        .h(px(280.0))
                        .overflow_y_scrollbar()
                        .child(cmp::app_list_panel(cx, &entries))
                        .into_any_element(),
                ],
            )
        })
        .child({
            let entries: Vec<TitleEntryView> = vm
                .top_titles
                .iter()
                .map(|e| TitleEntryView {
                    rank: e.rank,
                    app_class: e.app_class.clone(),
                    title: e.title.clone(),
                    total_millis: e.total_millis,
                    percentage: e.percentage,
                })
                .collect();
            card(
                cx,
                Some("Top Titles"),
                vec![
                    div()
                        .h(px(280.0))
                        .overflow_y_scrollbar()
                        .child(cmp::title_list_panel(cx, &entries))
                        .into_any_element(),
                ],
            )
        })
        .when(!vm.block_cards.is_empty(), |el| {
            el.child(card(
                cx,
                Some("Currently Blocked"),
                vm.block_cards
                    .iter()
                    .map(|c| block_card(cx, c))
                    .collect::<Vec<_>>(),
            ))
        })
}
