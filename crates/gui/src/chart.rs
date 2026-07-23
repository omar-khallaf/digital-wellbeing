//! Shared chart rendering components used by Dashboard and Reports screens.
//! Types defined here so both screens use the same data structures.

use chrono::NaiveDate;
use gpui::prelude::*;
use gpui::px;
use gpui::*;
use gpui_component::chart::{BarChart, PieChart};
use gpui_component::{h_flex, v_flex};

use crate::components::format_duration;
use crate::theme::{self, color_from_str, rad, sp};

/// One bar in the daily screen-time bar chart.
#[derive(Debug, Clone)]
pub struct Bar {
    pub date: NaiveDate,
    pub total_millis: f64,
}

/// One slice in a usage breakdown pie chart.
#[derive(Debug, Clone)]
pub struct Slice {
    pub app_id: String,
    pub display_name: String,
    pub color: String,
    pub percentage: f64,
}

/// Render a bar chart for daily screen time with formatted labels.
pub fn daily_bar_chart(cx: &App, bars: &[Bar]) -> AnyElement {
    if bars.is_empty() {
        return empty_state(cx, "No usage data for the selected range.").into_any_element();
    }

    let accent = theme::accent(cx);
    div()
        .h(px(210.0))
        .overflow_hidden()
        .child(
            BarChart::new(bars.to_vec())
                .band(|b: &Bar| b.date.format("%m/%d").to_string())
                .value(|b: &Bar| b.total_millis)
                .label(|b: &Bar| SharedString::from(format_duration(b.total_millis as i64)))
                .fill(move |_b, _bar, _chart, _align| accent)
                .label_axis(true),
        )
        .into_any_element()
}

/// Return the best display label for a slice: display_name if non-empty, else app_id.
fn slice_label(s: &Slice) -> &str {
    if s.display_name.is_empty() {
        &s.app_id
    } else {
        &s.display_name
    }
}

/// Render a donut pie chart with optional legend below it.
pub fn pie_chart_panel(cx: &App, slices: &[Slice], show_legend: bool) -> AnyElement {
    if slices.is_empty() {
        return div()
            .h(px(338.0))
            .items_center()
            .justify_center()
            .child(
                div()
                    .text_sm()
                    .text_color(theme::chart_text(cx))
                    .child("No data.".to_string()),
            )
            .into_any_element();
    }

    let chart = div()
        .h(px(230.0))
        .overflow_hidden()
        .child(
            PieChart::new(slices.to_vec())
                .value(|s: &Slice| s.percentage as f32)
                .color(|s: &Slice| color_from_str(&s.display_name))
                .inner_radius(45.)
                .outer_radius(80.)
                .label(|s: &Slice| {
                    SharedString::from(format!("{} {:.0}%", slice_label(s), s.percentage))
                }),
        )
        .into_any_element();

    if !show_legend {
        return chart;
    }

    let legend_items: Vec<AnyElement> = slices
        .iter()
        .map(|s| {
            let color = color_from_str(&s.display_name);
            h_flex()
                .gap_2()
                .items_center()
                .w_full()
                .child(div().size(px(10.0)).rounded(rad::full()).bg(color))
                .child(
                    div()
                        .flex_1()
                        .overflow_x_hidden()
                        .text_xs()
                        .text_color(theme::chart_text(cx))
                        .child(format!("{}  {:.0}%", slice_label(s), s.percentage)),
                )
                .into_any_element()
        })
        .collect();

    let legend = div()
        .id("chart-legend")
        .w_full()
        .max_h(px(110.0))
        .overflow_y_scroll()
        .overflow_x_hidden()
        .child(v_flex().gap_1().children(legend_items))
        .into_any_element();

    v_flex()
        .gap_2()
        .child(chart)
        .child(legend)
        .into_any_element()
}

/// Centered empty-state placeholder shown when no chart data is available.
pub fn empty_state(cx: &App, message: &str) -> AnyElement {
    div()
        .py(sp::LG)
        .w_full()
        .items_center()
        .justify_center()
        .child(
            div()
                .text_sm()
                .text_color(theme::text_muted(cx))
                .child(message.to_string()),
        )
        .into_any_element()
}
