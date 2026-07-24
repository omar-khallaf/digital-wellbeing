//! Shared chart rendering components used by Dashboard and Reports screens.
//! Types defined here so both screens use the same data structures.

use chrono::NaiveDate;
use gpui::prelude::*;
use gpui::px;
use gpui::*;
use gpui_component::chart::PieChart;
use gpui_component::plot::{
    AxisLabelSide, AxisText, Grid, PlotAxis,
    label::TEXT_SIZE,
    origin_point,
    scale::{Scale, ScaleBand, ScaleLinear},
    shape::{Bar as PlotBar, BarAlignment},
};
use gpui_component::scroll::ScrollableElement;
use gpui_component::{h_flex, v_flex};

use crate::components::format_duration;
use crate::theme::{self, color_from_str, rad, sp};

/// Trait abstracting over bar-like data so `daily_bar_chart` works with
/// types like `DailyBar` (reports).
pub trait HasBarData {
    fn date(&self) -> NaiveDate;
    fn total_millis(&self) -> f64;
}

/// One slice in a usage breakdown pie chart.
#[derive(Debug, Clone)]
pub struct Slice {
    pub app_id: String,
    pub display_name: String,
    pub color: String,
    pub percentage: f64,
}

/// Custom element that paints a bar chart with a fixed 0-24h Y-axis and hourly labels.
struct DailyBarChartElement<T: HasBarData + Clone> {
    data: Vec<T>,
    accent: Hsla,
    muted: Hsla,
    border: Hsla,
}

impl<T: HasBarData + Clone + 'static> IntoElement for DailyBarChartElement<T> {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

impl<T: HasBarData + Clone + 'static> Element for DailyBarChartElement<T> {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let style = gpui::Style {
            size: gpui::Size::full(),
            ..Default::default()
        };
        (window.request_layout(style, None, cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        _: &mut Window,
        _: &mut App,
    ) -> Self::PrepaintState {
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        _: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let accent = self.accent;
        let muted = self.muted;
        let border = self.border;

        let total_w = bounds.size.width.as_f32().max(1.0);
        let total_h = bounds.size.height.as_f32().max(1.0);

        let y_label_w = 30.0;
        let bottom_gap = 42.0;
        let top_margin = 24.0;
        let axis_gap = 20.0;
        let axis_x = y_label_w;
        let chart_l = axis_x + axis_gap; // bars domain start
        let chart_r = (total_w - 12.0).max(chart_l + 1.0); // right padding so last label isn't clipped
        let chart_b = total_h - bottom_gap;
        let chart_t = top_margin;

        // Y scale: 0–24 hours
        let y_scale = ScaleLinear::new(vec![0.0, 24.0], vec![chart_b, chart_t]);

        // X scale: band scale for date labels.
        let bands: Vec<String> = self
            .data
            .iter()
            .map(|d| d.date().format("%m/%d").to_string())
            .collect();
        let x_scale = ScaleBand::new(bands, vec![chart_l, chart_r])
            .padding_inner(0.3)
            .padding_outer(0.5); // Outer padding keeps first/last bar centers away from edges
        let band_w = x_scale.band_width();

        // ── Grid lines (horizontal, one per y-axis label) ───────────────
        let grid_y: Vec<Pixels> = (2..=24)
            .step_by(2)
            .map(|h| px(y_scale.tick(&(h as f64)).unwrap_or(0.0)))
            .collect();
        Grid::new()
            .stroke(border)
            .dash_array(&[px(4.), px(2.)])
            .y(grid_y)
            .paint(&bounds, window);

        // ── Y-axis (left) with hourly labels ────────────────────────────
        let label_offset = 8.0;
        let y_labels: Vec<AxisText> = (2..=24)
            .step_by(2)
            .map(|h| {
                let y = y_scale.tick(&(h as f64)).unwrap_or(0.0);
                AxisText::new(format!("{}h", h), px(y - label_offset), muted)
                    .align(TextAlign::Right)
            })
            .collect();
        PlotAxis::new()
            .stroke(border)
            .x_axis(false)
            .y(px(axis_x))
            .y_label_side(AxisLabelSide::Start)
            .y_label(y_labels)
            .paint(&bounds, window, cx);

        // ── X-axis (bottom) with date + duration labels ────────────────
        PlotAxis::new()
            .stroke(border)
            .y_axis(false)
            .x(px(chart_b))
            .x_label(vec![])
            .paint(&bounds, window, cx);

        // Manual two-line labels: date on line 1, formatted duration on line 2.
        let font = window.text_style().font();
        let font_size = px(TEXT_SIZE);
        let label_y_date = chart_b + TEXT_SIZE + 2.0;
        let label_y_duration = label_y_date + TEXT_SIZE + 1.0;
        let label_style = LabelStyle {
            font: &font,
            font_size,
            color: muted,
        };

        for datum in self.data.iter() {
            let day_str = datum.date().format("%m/%d").to_string();
            let Some(x_left) = x_scale.tick(&day_str) else {
                continue;
            };
            let bar_cx = px(x_left + band_w / 2.0);

            // Date label (line 1)
            paint_label(
                &day_str,
                bar_cx,
                label_y_date,
                &label_style,
                bounds,
                window,
                cx,
            );

            // Duration label (line 2)
            let total_millis = datum.total_millis() as i64;
            paint_label(
                &format_duration(total_millis),
                bar_cx,
                label_y_duration,
                &label_style,
                bounds,
                window,
                cx,
            );
        }

        // ── Bars ────────────────────────────────────────────────────────
        let y_for_base = y_scale.clone();
        let y_for_value = y_scale.clone();
        let x_for_cross = x_scale.clone();

        PlotBar::new()
            .data(self.data.clone())
            .alignment(BarAlignment::Bottom)
            .band_width(band_w)
            .cross(move |d| {
                let label = d.date().format("%m/%d").to_string();
                x_for_cross.tick(&label)
            })
            .base(move |_| y_for_base.tick(&0.0).unwrap_or(chart_b))
            .value(move |d| {
                let hours = d.total_millis() / 3_600_000.0;
                let clamped = hours.clamp(0.0, 24.0);
                y_for_value.tick(&clamped)
            })
            .fill(move |_, _, _| accent)
            .paint(&bounds, window, cx);
    }
}

/// Text styling for chart labels.
struct LabelStyle<'a> {
    font: &'a Font,
    font_size: Pixels,
    color: Hsla,
}

/// Paint a single text label centered above a bar in the chart.
fn paint_label(
    text: &str,
    bar_cx: Pixels,
    label_y: f32,
    style: &LabelStyle,
    bounds: Bounds<Pixels>,
    window: &mut Window,
    cx: &mut App,
) {
    let text: SharedString = text.into();
    let run = TextRun {
        len: text.len(),
        font: style.font.clone(),
        color: style.color,
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let shaped = window
        .text_system()
        .shape_line(text, style.font_size, &[run], None);
    let x = bar_cx - shaped.width() / 2.0;
    let origin = origin_point(x, px(label_y), bounds.origin);
    let _ = shaped.paint(origin, style.font_size, TextAlign::Left, None, window, cx);
}

/// Render a bar chart for daily screen time with a fixed 0-24h Y-axis
/// and hourly labels ("0h" … "24h").
pub fn daily_bar_chart<T: HasBarData + Clone + 'static>(cx: &App, bars: &[T]) -> AnyElement {
    if bars.is_empty() {
        return empty_state(cx, "No usage data for the selected range.");
    }

    let chart = DailyBarChartElement {
        data: bars.to_vec(),
        accent: theme::primary(cx),
        muted: theme::text_muted(cx),
        border: theme::border(cx),
    };

    let min_bar_width = 54.0;
    let content_width = bars.len() as f64 * min_bar_width;

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
                    SharedString::from(format!("{} {:.1}%", slice_label(s), s.percentage))
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
                        .child(format!("{}  {:.1}%", slice_label(s), s.percentage)),
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
