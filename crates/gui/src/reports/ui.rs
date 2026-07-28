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

use crate::chart::daily_bar_chart;
use crate::components::{
    self as cmp, RepAppsDelegate, RepTitlesDelegate, card, format_duration, time_range_selector,
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

pub fn render_reports_view(
    cx: &App,
    vm: &ReportsViewModel,
    apps_list: &Entity<ListState<RepAppsDelegate>>,
    titles_list: &Entity<ListState<RepTitlesDelegate>>,
    date_opts: DateRangeOptions,
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
                    date_opts.show_custom,
                    date_opts.custom_start_input,
                    date_opts.custom_end_input,
                    date_opts.on_preset,
                    date_opts.on_toggle_custom,
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
            vec![
                div()
                    .h(px(280.0))
                    .child(cmp::List::new(apps_list))
                    .into_any_element(),
            ],
        ))
        .child(card(
            cx,
            Some("All Titles"),
            vec![
                div()
                    .h(px(280.0))
                    .child(cmp::List::new(titles_list))
                    .into_any_element(),
            ],
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
