//! Dashboard gpui rendering — all render logic, no D-Bus/data access.
//! Consumes `DashboardViewModel` from `domain.rs` built by `data.rs`.

mod charts;
mod panels;

pub use self::charts::*;
pub use self::panels::*;

use gpui::prelude::*;
use gpui::px;
use gpui::*;
use gpui_component::list::ListState;
use gpui_component::{h_flex, v_flex};

use crate::chart::{EmptyState, PieChartPanel};
use crate::components::{
    self as cmp, Card, DashAppsDelegate, DashTitlesDelegate, StatCard, format_duration,
};
use crate::theme;

use super::domain::DashboardViewModel;
use super::viewmodel::compute_kpis;

/// Rendered dashboard view — owns all data needed for rendering.
pub struct DashboardView {
    pub vm: DashboardViewModel,
    pub apps_list: Entity<ListState<DashAppsDelegate>>,
    pub titles_list: Entity<ListState<DashTitlesDelegate>>,
}

impl RenderOnce for DashboardView {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let kpis = compute_kpis(&self.vm);

        v_flex()
            .gap_4()
            .child(
                h_flex()
                    .gap_4()
                    .child(
                        RenderOnce::render(
                            StatCard {
                                value: format_duration(kpis.total_millis).into(),
                                label: "Total Screen Time".into(),
                                dot_color: Some(theme::primary(cx)),
                            },
                            window,
                            cx,
                        )
                        .into_any_element(),
                    )
                    .child(
                        RenderOnce::render(
                            StatCard {
                                value: kpis.top_app.clone().into(),
                                label: format!(
                                    "Top App \u{B7} {}",
                                    format_duration(kpis.top_app_millis)
                                )
                                .into(),
                                dot_color: Some(theme::secondary(cx)),
                            },
                            window,
                            cx,
                        )
                        .into_any_element(),
                    )
                    .child(
                        RenderOnce::render(
                            StatCard {
                                value: kpis.active_blocks.to_string().into(),
                                label: "Active Blocks".into(),
                                dot_color: Some(theme::danger(cx)),
                            },
                            window,
                            cx,
                        )
                        .into_any_element(),
                    ),
            )
            .child(
                RenderOnce::render(
                    Card {
                        title: Some("Day Timeline".into()),
                        children: vec![if let Some(tl) = &self.vm.day_timeline {
                            RenderOnce::render(
                                DayTimelineChart {
                                    timeline: tl.clone(),
                                },
                                window,
                                cx,
                            )
                            .into_any_element()
                        } else {
                            RenderOnce::render(
                                EmptyState {
                                    message: "No timeline data for this day.".into(),
                                },
                                window,
                                cx,
                            )
                            .into_any_element()
                        }],
                    },
                    window,
                    cx,
                )
                .into_any_element(),
            )
            .child(
                v_flex()
                    .gap_4()
                    .child(
                        RenderOnce::render(
                            Card {
                                title: Some("By App".into()),
                                children: vec![
                                    RenderOnce::render(
                                        PieChartPanel {
                                            slices: self.vm.pie_app.clone(),
                                            show_legend: true,
                                        },
                                        window,
                                        cx,
                                    )
                                    .into_any_element(),
                                ],
                            },
                            window,
                            cx,
                        )
                        .into_any_element(),
                    )
                    .child(
                        RenderOnce::render(
                            Card {
                                title: Some("By Category".into()),
                                children: vec![
                                    RenderOnce::render(
                                        PieChartPanel {
                                            slices: self.vm.pie_category.clone(),
                                            show_legend: true,
                                        },
                                        window,
                                        cx,
                                    )
                                    .into_any_element(),
                                ],
                            },
                            window,
                            cx,
                        )
                        .into_any_element(),
                    ),
            )
            .child(
                RenderOnce::render(
                    Card {
                        title: Some("Top Apps".into()),
                        children: vec![
                            div()
                                .h(px(280.0))
                                .child(cmp::List::new(&self.apps_list))
                                .into_any_element(),
                        ],
                    },
                    window,
                    cx,
                )
                .into_any_element(),
            )
            .child(
                RenderOnce::render(
                    Card {
                        title: Some("Top Titles".into()),
                        children: vec![
                            div()
                                .h(px(280.0))
                                .child(cmp::List::new(&self.titles_list))
                                .into_any_element(),
                        ],
                    },
                    window,
                    cx,
                )
                .into_any_element(),
            )
            .when(!self.vm.block_cards.is_empty(), |el| {
                el.child(
                    RenderOnce::render(
                        Card {
                            title: Some("Currently Blocked".into()),
                            children: self
                                .vm
                                .block_cards
                                .iter()
                                .map(|c| {
                                    RenderOnce::render(BlockCard { info: c.clone() }, window, cx)
                                        .into_any_element()
                                })
                                .collect::<Vec<_>>(),
                        },
                        window,
                        cx,
                    )
                    .into_any_element(),
                )
            })
    }
}

/// Render the complete dashboard view from a ViewModel.
/// The `apps_list` / `titles_list` are gpui-component `List` states
/// (virtualised, culled), owned by the parent `App` entity.
pub fn render_dashboard_view(
    window: &mut Window,
    cx: &mut App,
    vm: &DashboardViewModel,
    apps_list: &Entity<ListState<DashAppsDelegate>>,
    titles_list: &Entity<ListState<DashTitlesDelegate>>,
) -> impl IntoElement {
    RenderOnce::render(
        DashboardView {
            vm: vm.clone(),
            apps_list: apps_list.clone(),
            titles_list: titles_list.clone(),
        },
        window,
        cx,
    )
}
