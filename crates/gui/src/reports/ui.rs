//! Reports gpui rendering — all render logic, no D-Bus/data access.
//! Consumes `ReportsViewModel` from `domain.rs` built by `data.rs`.
//! Chart rendering reuses shared components from `crate::chart`.

use gpui::prelude::*;
use gpui::px;
use gpui::*;
use gpui_component::ActiveTheme;
use gpui_component::button::Button;
use gpui_component::input::InputState;
use gpui_component::{h_flex, v_flex};
use wellbeing_core::DateRange;

use crate::chart::{daily_bar_chart, empty_state};
use crate::components::{card, format_duration, time_range_selector};
use crate::theme::{self, rad, sp};

use super::domain::{ReportAppEntry, ReportsViewModel};

/// Render the reports view from a ViewModel.
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
        .child(card(
            cx,
            Some("All Apps"),
            vec![app_list_panel(cx, &vm.app_list).into_any_element()],
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

fn app_list_panel(cx: &App, entries: &[ReportAppEntry]) -> AnyElement {
    if entries.is_empty() {
        return empty_state(cx, "No usage data yet.").into_any_element();
    }

    let rows: Vec<AnyElement> = entries
        .iter()
        .map(|entry| {
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
