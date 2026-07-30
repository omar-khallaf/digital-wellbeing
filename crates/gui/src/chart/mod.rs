//! Shared chart rendering components used by Dashboard and Reports screens.
//! Types defined here so both screens use the same data structures.

use chrono::NaiveDate;
use gpui::prelude::*;
use gpui::{AnyElement, App, Hsla, SharedString, Window, div, px};
use gpui_component::chart::PieChart;
use gpui_component::scroll::ScrollableElement;
use gpui_component::{h_flex, v_flex};

use crate::theme::{self, color_from_str, rad, sp};

mod element;
use self::element::DailyBarChartElement;

/// Trait abstracting over bar-like data so `daily_bar_chart` works with
/// types like `DailyBar` (reports).
pub trait HasBarData {
    fn date(&self) -> NaiveDate;
    fn total_millis(&self) -> f64;
}

/// One slice in a usage breakdown pie chart.
#[derive(Debug, Clone, PartialEq)]
pub struct Slice {
    pub app_class: String,
    pub display_name: String,
    pub color: String,
    pub percentage: f64,
}

/// Render a bar chart for daily screen time with a fixed 0-24h Y-axis
/// and hourly labels ("0h" … "24h").
pub struct DailyBarChart<T: HasBarData + Clone + 'static> {
    pub data: Vec<T>,
    pub accent: Hsla,
    pub muted: Hsla,
    pub border: Hsla,
}

impl<T: HasBarData + Clone + 'static> RenderOnce for DailyBarChart<T> {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        if self.data.is_empty() {
            return EmptyState {
                message: "No usage data for the selected range.".into(),
            }
            .render(window, cx)
            .into_any_element();
        }

        let chart = DailyBarChartElement {
            data: self.data,
            accent: self.accent,
            muted: self.muted,
            border: self.border,
        };

        let min_bar_width = 54.0;
        let content_width = chart.data.len() as f64 * min_bar_width;

        // Enable horizontal scrolling when content exceeds ~650px (e.g. 14d, 30d, etc.)
        // to preserve 54px columns and prevent labels from squeezing into each other.
        if content_width > 650.0 {
            div()
                .h(px(300.0))
                .overflow_x_scrollbar()
                .child(div().w(px(content_width as f32)).h_full().child(chart))
                .into_any_element()
        } else {
            div().h(px(300.0)).child(chart).into_any_element()
        }
    }
}

/// Render a donut pie chart with optional legend below it.
pub struct PieChartPanel {
    pub slices: Vec<Slice>,
    pub show_legend: bool,
}

impl PieChartPanel {
    /// Return the best display label for a slice: display_name if non-empty, else app_class.
    fn slice_label(s: &Slice) -> &str {
        if s.display_name.is_empty() {
            &s.app_class
        } else {
            &s.display_name
        }
    }
}

impl RenderOnce for PieChartPanel {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        if self.slices.is_empty() {
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
                PieChart::new(self.slices.to_vec())
                    .value(|s: &Slice| s.percentage as f32)
                    .color(|s: &Slice| color_from_str(&s.display_name))
                    .inner_radius(45.)
                    .outer_radius(80.)
                    .label(|s: &Slice| {
                        SharedString::from(format!("{} {:.1}%", Self::slice_label(s), s.percentage))
                    }),
            )
            .into_any_element();

        if !self.show_legend {
            return chart;
        }

        let legend_items: Vec<AnyElement> = self
            .slices
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
                            .child(format!("{}  {:.1}%", Self::slice_label(s), s.percentage)),
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
}

/// Centered empty-state placeholder shown when no chart data is available.
pub struct EmptyState {
    pub message: SharedString,
}

impl RenderOnce for EmptyState {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        div()
            .py(sp::LG)
            .w_full()
            .items_center()
            .justify_center()
            .child(
                div()
                    .text_sm()
                    .text_color(theme::text_muted(cx))
                    .child(self.message),
            )
            .into_any_element()
    }
}
