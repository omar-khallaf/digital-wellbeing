//! Dashboard gpui rendering — all render logic, no D-Bus/data access.
//! Consumes `DashboardViewModel` from `domain.rs` built by `data.rs`.

mod charts;
mod panels;

pub use self::charts::*;
// Internal helpers are pub(super) in sub-modules and imported here,
// not re-exported (they were private in the original single-file layout).
use self::panels::{app_list_panel, block_card, title_list_panel};

use gpui::prelude::*;
use gpui::*;
use gpui_component::{h_flex, v_flex};

use crate::chart::{empty_state, pie_chart_panel};
use crate::components::{card, format_duration, stat_card};
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
        .child(card(
            cx,
            Some("Top Titles"),
            vec![title_list_panel(cx, &vm.top_titles).into_any_element()],
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
