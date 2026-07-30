//! Date-range selector component — preset buttons + custom range inputs.

use chrono::NaiveDate;
use std::sync::Arc;

use gpui::prelude::*;
use gpui::px;
use gpui::*;
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::h_flex;
use gpui_component::input::{Input, InputState};
use wellbeing_core::DateRange;

use crate::theme::{sp, text_muted};

/// Callback for date range selection changes.
type OnChangeFn = Arc<dyn Fn(DateRange, &mut App)>;
/// Callback for toggle actions (e.g. show/hide custom range).
type OnToggleFn = Arc<dyn Fn(&mut App)>;

/// Time range selector with preset buttons (7d, 14d, 30d, 90d), a custom
/// date range toggle, and date input fields for arbitrary ranges.
///
/// When `show_custom` is true, two date text inputs and an "Apply" button
/// are shown alongside the presets.
///
/// `InputState` entities must be created by the caller (parent) since
/// creation requires `&mut Window`.
pub struct TimeRangeSelector {
    pub selected: DateRange,
    pub show_custom: bool,
    pub custom_start_input: Option<Entity<InputState>>,
    pub custom_end_input: Option<Entity<InputState>>,
    pub on_change: OnChangeFn,
    pub on_toggle_custom: OnToggleFn,
}

impl RenderOnce for TimeRangeSelector {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let preset_specs: &[(&str, &str, u32)] = &[
            ("7d", "7d", 7),
            ("14d", "14d", 14),
            ("30d", "30d", 30),
            ("90d", "90d", 90),
        ];

        let preset_buttons: Vec<AnyElement> = preset_specs
            .iter()
            .map(|&(id, label, days)| {
                let oc = self.on_change.clone();
                let mut btn = Button::new(id).label(label);
                if !self.show_custom && self.selected == DateRange::last_n_days(days) {
                    btn = btn.primary();
                }
                btn.on_click(move |_, _, cx| (oc.as_ref())(DateRange::last_n_days(days), cx))
                    .into_any_element()
            })
            .collect();

        let btn_custom = {
            let oc = self.on_toggle_custom.clone();
            let mut btn = Button::new("custom-range").label("Custom");
            if self.show_custom {
                btn = btn.primary();
            }
            btn.on_click(move |_, _, cx| (oc.as_ref())(cx))
        };

        let custom_inputs: Option<AnyElement> = if self.show_custom {
            Some(
                h_flex()
                    .gap_1()
                    .items_center()
                    .child(
                        Input::new(
                            self.custom_start_input
                                .as_ref()
                                .expect("custom_start_input is None"),
                        )
                        .w(px(150.0)),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(text_muted(cx))
                            .px(sp::XS)
                            .child("to"),
                    )
                    .child(
                        Input::new(
                            self.custom_end_input
                                .as_ref()
                                .expect("custom_end_input is None"),
                        )
                        .w(px(150.0)),
                    )
                    .child({
                        let oc = self.on_change.clone();
                        let start_clone = self.custom_start_input.clone();
                        let end_clone = self.custom_end_input.clone();
                        Button::new("apply-custom")
                            .label("Apply")
                            .primary()
                            .on_click(move |_, _, app| {
                                let start_str = start_clone
                                    .as_ref()
                                    .map(|e| e.read(app).value().to_string())
                                    .unwrap_or_default();
                                let end_str = end_clone
                                    .as_ref()
                                    .map(|e| e.read(app).value().to_string())
                                    .unwrap_or_default();
                                if let (Ok(start_date), Ok(end_date)) = (
                                    NaiveDate::parse_from_str(&start_str, "%Y-%m-%d"),
                                    NaiveDate::parse_from_str(&end_str, "%Y-%m-%d"),
                                ) && start_date <= end_date
                                {
                                    (oc.as_ref())(
                                        DateRange {
                                            start: start_date,
                                            end: end_date,
                                        },
                                        app,
                                    );
                                }
                            })
                    })
                    .into_any(),
            )
        } else {
            None
        };

        h_flex()
            .gap_2()
            .children(preset_buttons)
            .child(btn_custom)
            .when_some(custom_inputs, |el, inputs| el.child(inputs))
            .into_any_element()
    }
}
