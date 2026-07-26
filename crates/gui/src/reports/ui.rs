//! Reports gpui rendering — all render logic, no D-Bus/data access.
//! Consumes `ReportsViewModel` from `domain.rs` built by `data.rs`.
//! Chart rendering reuses shared components from `crate::chart`.

use gpui::prelude::*;
use gpui::px;
use gpui::*;
use gpui_component::ActiveTheme;
use gpui_component::button::Button;
use gpui_component::input::InputState;
use gpui_component::scroll::ScrollableElement;
use gpui_component::{h_flex, v_flex};
use wellbeing_core::DateRange;

use crate::chart::daily_bar_chart;
use crate::components::{
    self as cmp, AppEntryView, TitleEntryView, card, format_duration, time_range_selector,
};
use crate::theme::{self, rad, sp};

use super::domain::ReportsViewModel;

pub fn render_reports_view(
    cx: &App,
    vm: &ReportsViewModel,
    show_custom: bool,
    custom_start_input: Option<Entity<InputState>>,
    custom_end_input: Option<Entity<InputState>>,
    on_preset: impl Fn(DateRange, &mut App) + 'static,
    on_toggle_custom: impl Fn(&mut App) + 'static,
) -> impl IntoElement {
    v_flex()
        .gap_4()
        .child(
            h_flex()
                .gap_3()
                .items_center()
                .child(time_range_selector(
                    cx,
                    vm.date_range,
                    show_custom,
                    custom_start_input,
                    custom_end_input,
                    on_preset,
                    on_toggle_custom,
                ))
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
                            vm.date_range.start.format("%b %d"),
                            vm.date_range.end.format("%b %d, %Y"),
                        )),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(theme::chart_text(cx))
                        .child(format!(
                            "Total {} \u{B7} Top app {}",
                            format_duration(vm.total_millis),
                            vm.top_app
                        )),
                ),
        )
        .child(card(
            cx,
            Some("Daily Screen Time"),
            vec![daily_bar_chart(cx, &vm.bar_chart).into_any_element()],
        ))
        .child({
            let entries: Vec<AppEntryView> = vm
                .app_list
                .iter()
                .map(|e| AppEntryView {
                    rank: e.rank,
                    display_name: e.display_name.clone(),
                    total_millis: e.total_millis,
                    percentage: e.percentage,
                    dot_color: None,
                    badge: None,
                })
                .collect();
            card(
                cx,
                Some("All Apps"),
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
                .title_list
                .iter()
                .map(|e| TitleEntryView {
                    rank: e.rank,
                    app_id: e.app_id.clone(),
                    title: e.title.clone(),
                    total_millis: e.total_millis,
                    percentage: e.percentage,
                })
                .collect();
            card(
                cx,
                Some("All Titles"),
                vec![
                    div()
                        .h(px(280.0))
                        .overflow_y_scrollbar()
                        .child(cmp::title_list_panel(cx, &entries))
                        .into_any_element(),
                ],
            )
        })
        .child(
            h_flex()
                .gap_2()
                .child(
                    Button::new("export-csv")
                        .label("Export CSV")
                        .on_click(|_, _, _| {
                            tracing::info!("reports: export CSV requested (serialized from cache)");
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
