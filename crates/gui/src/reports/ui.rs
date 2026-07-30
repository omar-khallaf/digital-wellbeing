//! Reports gpui rendering — all render logic, no D-Bus/data access.
//! Consumes `ReportsViewModel` from `domain.rs` built by `data.rs`.
//! Chart rendering reuses shared components from `crate::chart`.

use gpui::prelude::*;
use gpui::px;
use gpui::*;
use gpui_component::ActiveTheme;
use gpui_component::button::Button;
use gpui_component::list::ListState;
use gpui_component::{h_flex, v_flex};

use crate::chart::DailyBarChart;
use crate::components::{
    self as cmp, Card, RepAppsDelegate, RepTitlesDelegate, TimeRangeSelector, format_duration,
};
use crate::theme::{self, rad, sp};

use super::domain::ReportsViewModel;

type PresetHandler = Box<dyn Fn(wellbeing_core::DateRange, &mut gpui::App)>;

/// Bundled options for the date-range selector to keep argument count ≤ 7.
pub struct DateRangeOptions {
    pub show_custom: bool,
    pub custom_start_input: Option<Entity<gpui_component::input::InputState>>,
    pub custom_end_input: Option<Entity<gpui_component::input::InputState>>,
    pub on_preset: PresetHandler,
    pub on_toggle_custom: Box<dyn Fn(&mut gpui::App)>,
}

/// Rendered reports view — owns all data needed for rendering.
pub struct ReportsView {
    pub vm: ReportsViewModel,
    pub apps_list: Entity<ListState<RepAppsDelegate>>,
    pub titles_list: Entity<ListState<RepTitlesDelegate>>,
    pub date_opts: DateRangeOptions,
}

impl RenderOnce for ReportsView {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        v_flex()
            .gap_4()
            .child(
                h_flex()
                    .gap_3()
                    .items_center()
                    .child(
                        RenderOnce::render(
                            TimeRangeSelector {
                                selected: self.vm.date_range,
                                show_custom: self.date_opts.show_custom,
                                custom_start_input: self.date_opts.custom_start_input,
                                custom_end_input: self.date_opts.custom_end_input,
                                on_change: std::sync::Arc::from(self.date_opts.on_preset),
                                on_toggle_custom: std::sync::Arc::from(
                                    self.date_opts.on_toggle_custom,
                                ),
                            },
                            window,
                            cx,
                        )
                        .into_any_element(),
                    )
                    .child(
                        div()
                            .text_xs()
                            .px(sp::XS)
                            .py(px(2.0))
                            .rounded(rad::sm())
                            .bg(theme::accent(cx))
                            .text_color(cx.theme().accent_foreground)
                            .child(format!(
                                "{} \u{2013} {}",
                                self.vm.date_range.start.format("%b %d"),
                                self.vm.date_range.end.format("%b %d, %Y"),
                            )),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme::chart_text(cx))
                            .child(format!(
                                "Total {} \u{B7} Top app {}",
                                format_duration(self.vm.total_millis),
                                self.vm.top_app
                            )),
                    ),
            )
            .child(
                RenderOnce::render(
                    Card {
                        title: Some("Daily Screen Time".into()),
                        children: vec![
                            RenderOnce::render(
                                DailyBarChart {
                                    data: self.vm.bar_chart.clone(),
                                    accent: theme::primary(cx),
                                    muted: theme::chart_text(cx),
                                    border: theme::border(cx),
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
                        title: Some("All Apps".into()),
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
                        title: Some("All Titles".into()),
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
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("export-csv")
                            .label("Export CSV")
                            .on_click(|_, _, _| {
                                tracing::info!(
                                    "reports: export CSV requested (serialized from cache)"
                                );
                            }),
                    )
                    .child(
                        Button::new("export-json")
                            .label("Export JSON")
                            .on_click(|_, _, _| {
                                tracing::info!(
                                    "reports: export JSON requested (serialized from cache)"
                                );
                            }),
                    ),
            )
    }
}

pub fn render_reports_view(
    window: &mut Window,
    cx: &mut App,
    vm: &ReportsViewModel,
    apps_list: &Entity<ListState<RepAppsDelegate>>,
    titles_list: &Entity<ListState<RepTitlesDelegate>>,
    date_opts: DateRangeOptions,
) -> impl IntoElement {
    RenderOnce::render(
        ReportsView {
            vm: vm.clone(),
            apps_list: apps_list.clone(),
            titles_list: titles_list.clone(),
            date_opts,
        },
        window,
        cx,
    )
}
