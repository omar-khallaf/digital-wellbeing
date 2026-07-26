//! Reusable UI primitives built on `gpui_component`.
//!
//! These wrap the component library's styling so every screen shares one
//! visual language: a `Card` panel, a `StatCard` KPI tile, and a `SectionTitle`.
//! Icons are rendered as theme-aware glyphs (no external font dependency).

use chrono::NaiveDate;
use gpui::prelude::*;
use gpui::px;
use gpui::*;
use gpui_component::input::{Input, InputState};
use gpui_component::{button::Button, button::ButtonVariants, h_flex, v_flex};
use wellbeing_core::DateRange;

use crate::chart::empty_state;
use crate::theme;
use crate::theme::*;

fn card_shadow() -> Vec<BoxShadow> {
    vec![BoxShadow {
        color: hsla(0.0, 0.0, 0.0, 0.25),
        offset: gpui::Point {
            x: px(0.0),
            y: px(1.0),
        },
        blur_radius: px(3.0),
        spread_radius: px(0.0),
        inset: false,
    }]
}

/// A titled, bordered surface panel.
///
/// `title` of `None` renders a padded card (useful for chart containers).
/// `title` of `Some(..)` adds a small caption header above the children.
pub fn card(
    cx: &App,
    title: Option<&str>,
    children: impl IntoIterator<Item = AnyElement>,
) -> AnyElement {
    let kids: Vec<AnyElement> = children.into_iter().collect();

    let body = match title {
        Some(t) => v_flex()
            .gap_2()
            .child(section_title(cx, t))
            .children(kids)
            .into_any_element(),
        None => v_flex().gap_2().children(kids).into_any_element(),
    };

    div()
        .bg(surface(cx))
        .border_1()
        .border_color(border(cx))
        .rounded(rad::lg())
        .p(sp::LG)
        .shadow(card_shadow())
        .overflow_hidden()
        .child(body)
        .into_any_element()
}

/// A KPI tile: small label + large value, with an optional accent dot.
///
/// The caller should pass a pre-adjusted dot color from [`theme::primary`],
/// [`theme::secondary`], [`theme::danger`], etc.
pub fn stat_card(cx: &App, value: &str, label: &str, dot: Option<Hsla>) -> AnyElement {
    let dot_el = dot.map(|c| {
        div()
            .size(px(8.0))
            .rounded(rad::full())
            .bg(c)
            .into_any_element()
    });

    div()
        .flex_1()
        .bg(surface(cx))
        .border_1()
        .border_color(border(cx))
        .rounded(rad::lg())
        .p(sp::LG)
        .shadow(card_shadow())
        .child(
            h_flex()
                .items_center()
                .gap_2()
                .when_some(dot_el, |el, dot| el.child(dot))
                .child(
                    div()
                        .text_xs()
                        .text_color(text_muted(cx))
                        .child(label.to_string()),
                ),
        )
        .child(
            div()
                .mt_1()
                .text_2xl()
                .font_weight(FontWeight::BOLD)
                .text_color(text_primary(cx))
                .child(value.to_string()),
        )
        .into_any_element()
}

/// Small semibold caption used above card content.
pub fn section_title(cx: &App, title: &str) -> AnyElement {
    div()
        .text_sm()
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(text_primary(cx))
        .child(title.to_string())
        .into_any_element()
}

/// Optional badge rendered after the display name (e.g. "BLOCKED").
#[derive(Debug, Clone)]
pub struct AppBadge {
    pub text: String,
    pub color: Hsla,
}

/// Shared data for a single app entry row — used by both the dashboard
/// "top apps" panel and the reports "all apps" panel.
///
/// Callers leave `dot_color` / `badge` as `None` when those extras
/// are not needed (reports uses neither; dashboard uses both).
#[derive(Debug, Clone)]
pub struct AppEntryView {
    pub rank: usize,
    pub display_name: String,
    pub total_millis: i64,
    pub percentage: f64,
    pub dot_color: Option<Hsla>,
    pub badge: Option<AppBadge>,
}

/// Renders a scrollable list of app entries.
///
/// Layout per row (mirrors `title_list_panel`):
/// ```text
/// [#rank] [●] [display_name] [BADGE] [duration  ]
///                                     [percentage]
/// ```
///
/// The colored dot and badge are only rendered when their respective
/// `Option` fields are `Some`.
pub fn app_list_panel(cx: &App, entries: &[AppEntryView]) -> AnyElement {
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
                .when_some(entry.dot_color, |el, color| {
                    el.child(div().size(px(10.0)).rounded(rad::full()).bg(color))
                })
                .child(
                    div()
                        .text_sm()
                        .flex_1()
                        .text_color(theme::text_primary(cx))
                        .child(entry.display_name.clone()),
                )
                .when_some(entry.badge.as_ref(), |el, badge| {
                    el.child(
                        div()
                            .text_xs()
                            .text_color(badge.color)
                            .child(badge.text.clone()),
                    )
                })
                .child(
                    v_flex()
                        .items_end()
                        .gap_0()
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::BOLD)
                                .text_color(theme::text_primary(cx))
                                .child(format_duration(entry.total_millis)),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(theme::text_label(cx))
                                .child(format!("{:.1}%", entry.percentage)),
                        ),
                )
                .into_any_element()
        })
        .collect();

    v_flex().gap_1().children(rows).into_any_element()
}

/// Shared data for a single title entry row — used by both the dashboard
/// "top titles" panel and the reports "all titles" panel.
#[derive(Debug, Clone)]
pub struct TitleEntryView {
    pub rank: usize,
    pub app_id: String,
    pub title: String,
    pub total_millis: i64,
    pub percentage: f64,
}

/// Renders a scrollable list of title entries.
///
/// Layout per row:
/// ```text
/// [#rank] [Title      ] [duration  ]
///         [app_id     ] [percentage]
/// ```
///
/// Callers may pass a `limit` to cap the number of visible entries
/// (the dashboard shows only the top few; reports shows all).
pub fn title_list_panel(cx: &App, entries: &[TitleEntryView]) -> AnyElement {
    if entries.is_empty() {
        return empty_state(cx, "No title usage data yet.").into_any_element();
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
                    v_flex()
                        .flex_1()
                        .gap_0()
                        .child(
                            div()
                                .text_sm()
                                .text_color(theme::text_primary(cx))
                                .child(entry.title.clone()),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(theme::text_muted(cx))
                                .child(entry.app_id.clone()),
                        ),
                )
                .child(
                    v_flex()
                        .items_end()
                        .gap_0()
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::BOLD)
                                .text_color(theme::text_primary(cx))
                                .child(format_duration(entry.total_millis)),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(theme::text_label(cx))
                                .child(format!("{:.1}%", entry.percentage)),
                        ),
                )
                .into_any_element()
        })
        .collect();

    v_flex().gap_1().children(rows).into_any_element()
}

/// Format milliseconds into a human-readable duration string.
pub fn format_duration(total_millis: i64) -> String {
    let total_minutes = (total_millis + 60000 - 1) / 60000;
    if total_minutes < 60 {
        format!("{}m", total_minutes)
    } else {
        let hours = total_minutes / 60;
        let mins = total_minutes % 60;
        if mins == 0 {
            format!("{}h", hours)
        } else {
            format!("{}h {}m", hours, mins)
        }
    }
}

/// Time range selector with preset buttons (1d, 7d, 14d, 30d, 90d), a custom
/// date range toggle, and date input fields for arbitrary ranges.
///
/// When `show_custom` is true, two date text inputs and an "Apply" button are
/// shown alongside the presets. The `on_change` callback fires whenever a
/// preset is clicked OR the custom "Apply" button constructs a valid DateRange.
///
/// InputState entities must be created by the caller (parent) since creation
/// requires `&mut Window`.
pub fn time_range_selector(
    cx: &App,
    selected: DateRange,
    show_custom: bool,
    custom_start_input: Option<Entity<InputState>>,
    custom_end_input: Option<Entity<InputState>>,
    on_change: impl Fn(DateRange, &mut App) + 'static,
    on_toggle_custom: impl Fn(&mut App) + 'static,
) -> AnyElement {
    let on_change = std::sync::Arc::new(on_change);
    let on_toggle_custom = std::sync::Arc::new(on_toggle_custom);

    let preset_specs: &[(&str, &str, u32)] = &[
        ("7d", "7d", 7),
        ("14d", "14d", 14),
        ("30d", "30d", 30),
        ("90d", "90d", 90),
    ];

    let preset_buttons: Vec<AnyElement> = preset_specs
        .iter()
        .map(|&(id, label, days)| {
            let oc = on_change.clone();
            let mut btn = Button::new(id).label(label);
            if !show_custom && selected == DateRange::last_n_days(days) {
                btn = btn.primary();
            }
            btn.on_click(move |_, _, cx| (oc.as_ref())(DateRange::last_n_days(days), cx))
                .into_any_element()
        })
        .collect();

    let btn_custom = {
        let mut btn = Button::new("custom-range").label("Custom");
        if show_custom {
            btn = btn.primary();
        }
        btn.on_click(move |_, _, cx| (on_toggle_custom.as_ref())(cx))
    };

    let custom_inputs: Option<AnyElement> = if show_custom {
        Some(
            h_flex()
                .gap_1()
                .items_center()
                .child(
                    Input::new(
                        custom_start_input
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
                    Input::new(custom_end_input.as_ref().expect("custom_end_input is None"))
                        .w(px(150.0)),
                )
                .child({
                    let oc = on_change.clone();
                    let start = custom_start_input.clone();
                    let end = custom_end_input.clone();
                    Button::new("apply-custom")
                        .label("Apply")
                        .primary()
                        .on_click(move |_, _, app| {
                            let start_str = start
                                .as_ref()
                                .map(|e| e.read(app).value().to_string())
                                .unwrap_or_default();
                            let end_str = end
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
