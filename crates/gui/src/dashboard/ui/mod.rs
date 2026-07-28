//! Dashboard gpui rendering — all render logic, no D-Bus/data access.
//! Consumes `DashboardViewModel` from `domain.rs` built by `data.rs`.

mod charts;
mod panels;

pub use self::charts::*;
use self::panels::block_card;

use gpui::prelude::*;
use gpui::px;
use gpui::*;
use gpui_component::list::ListState;
use gpui_component::{h_flex, v_flex};

use crate::chart::{empty_state, pie_chart_panel};
use crate::components::{
    self as cmp, DashAppsDelegate, DashTitlesDelegate, card, format_duration, stat_card,
};
use crate::theme;

use super::domain::DashboardViewModel;
use super::viewmodel::compute_kpis;

/// Render the complete dashboard view from a ViewModel.
/// The `apps_list` / `titles_list` are gpui-component `List` states
/// (virtualised, culled), owned by the parent `App` entity.
pub fn render_dashboard_view(
    cx: &App,
    vm: &DashboardViewModel,
    apps_list: &Entity<ListState<DashAppsDelegate>>,
    titles_list: &Entity<ListState<DashTitlesDelegate>>,
) -> impl IntoElement {
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
        .child(card(
            cx,
            Some("Top Apps"),
            vec![
                div()
                    .h(px(280.0))
                    .child(cmp::List::new(apps_list))
                    .into_any_element(),
            ],
        ))
        .child(card(
            cx,
            Some("Top Titles"),
            vec![
                div()
                    .h(px(280.0))
                    .child(cmp::List::new(titles_list))
                    .into_any_element(),
            ],
        ))
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
