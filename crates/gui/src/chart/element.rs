//! Private bar-chart element that paints a 0-24h Y-axis with hourly labels.
//! Data comes from the public [`DailyBarChart`](super::DailyBarChart) component.

use gpui::prelude::*;
use gpui::{
    App, Bounds, Element, ElementId, Font, GlobalElementId, Hsla, InspectorElementId, LayoutId,
    Pixels, SharedString, Size, Style, TextAlign, TextRun, Window, px,
};
use gpui_component::plot::{
    AxisLabelSide, AxisText, Grid, PlotAxis,
    label::TEXT_SIZE,
    origin_point,
    scale::{Scale, ScaleBand, ScaleLinear},
    shape::{Bar as PlotBar, BarAlignment},
};

use super::HasBarData;
use crate::components::format_duration;

/// Custom element that paints a bar chart with a fixed 0-24h Y-axis and hourly labels.
pub(super) struct DailyBarChartElement<T: HasBarData + Clone> {
    pub(super) data: Vec<T>,
    pub(super) accent: Hsla,
    pub(super) muted: Hsla,
    pub(super) border: Hsla,
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
        let style = Style {
            size: Size::full(),
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
        let chart_l = axis_x + axis_gap;
        let chart_r = (total_w - 12.0).max(chart_l + 1.0); // right padding so last label isn't clipped
        let chart_b = total_h - bottom_gap;
        let chart_t = top_margin;

        let y_scale = ScaleLinear::new(vec![0.0, 24.0], vec![chart_b, chart_t]);

        let bands: Vec<String> = self
            .data
            .iter()
            .map(|d| d.date().format("%m/%d").to_string())
            .collect();
        let x_scale = ScaleBand::new(bands, vec![chart_l, chart_r])
            .padding_inner(0.3)
            .padding_outer(0.5); // Outer padding keeps first/last bar centers away from edges
        let band_w = x_scale.band_width();

        let grid_y: Vec<Pixels> = (2..=24)
            .step_by(2)
            .map(|h| px(y_scale.tick(&(h as f64)).unwrap_or(0.0)))
            .collect();
        Grid::new()
            .stroke(border)
            .dash_array(&[px(4.), px(2.)])
            .y(grid_y)
            .paint(&bounds, window);

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

        PlotAxis::new()
            .stroke(border)
            .y_axis(false)
            .x(px(chart_b))
            .x_label(vec![])
            .paint(&bounds, window, cx);

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

            paint_label(
                &day_str,
                bar_cx,
                label_y_date,
                &label_style,
                bounds,
                window,
                cx,
            );

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

        // Bars
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

struct LabelStyle<'a> {
    font: &'a Font,
    font_size: Pixels,
    color: Hsla,
}

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
