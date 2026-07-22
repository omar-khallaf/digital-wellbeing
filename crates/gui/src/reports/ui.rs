//! Reports gpui rendering — all render logic, no D-Bus/data access.
//! Consumes `ReportsViewModel` from `domain.rs` built by `data.rs`.
//! Chart rendering reuses shared components from `crate::chart`.

use gpui::prelude::*;
use gpui::*;
use gpui_component::ActiveTheme;
use gpui_component::button::Button;
use gpui_component::{h_flex, v_flex};
use wellbeing_core::DateRange;

use crate::chart::{daily_bar_chart, pie_chart_panel};
use crate::components::{card, format_duration, time_range_selector};
use crate::theme::{self, rad, sp};

use super::domain::ReportsViewModel;

/// Render the reports view from a ViewModel.
pub fn render_reports_view(
    cx: &App,
    vm: &ReportsViewModel,
    on_range_change: impl Fn(DateRange, &mut App) + 'static,
) -> impl IntoElement {
    v_flex()
        .gap_4()
        .child(
            h_flex()
                .gap_3()
                .items_center()
                .child(time_range_selector(cx, vm.date_range, on_range_change))
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
                            format_duration(vm.total_minutes),
                            vm.top_app
                        )),
                ),
        )
        .child(card(
            cx,
            Some("Daily Screen Time"),
            vec![daily_bar_chart(cx, &vm.bar_chart).into_any_element()],
        ))
        .child(card(
            cx,
            Some("By App"),
            vec![pie_chart_panel(cx, &vm.pie_app, false).into_any_element()],
        ))
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
