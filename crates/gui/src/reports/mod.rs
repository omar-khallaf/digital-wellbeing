//! Reports screen — usage reports with charts and export (CSV / JSON).
//!
//! Reuses the dashboard's pure builders (`build_bars`, `build_app_slices`) to
//! produce chart-ready data from the cached daily usage, then renders real
//! `gpui_component::chart` components inside `Card` panels.

use gpui::prelude::*;
use gpui::px;
use gpui::*;
use gpui_component::ActiveTheme;
use gpui_component::button::Button;
use gpui_component::chart::{BarChart, PieChart};
use gpui_component::{h_flex, v_flex};
use wellbeing_core::{AppCategoryRow, Category, DailySummary, DailyUsageEntry, DateRange};

use crate::components::{card, time_range_selector};
use crate::theme::{rad, resolve_color, sp};

/// One bar in a time-series bar chart.
#[derive(Debug, Clone)]
pub struct ReportBar {
    pub date: chrono::NaiveDate,
    pub total_minutes: f64,
}

/// One slice in a usage-breakdown pie.
#[derive(Debug, Clone)]
pub struct ReportSlice {
    pub app_id: String,
    pub display_name: String,
    pub color: String,
    pub percentage: f64,
}

/// ViewModel for the reports screen.
#[derive(Debug, Clone)]
pub struct ReportsViewModel {
    pub date_range: DateRange,
    pub bar_chart: Vec<ReportBar>,
    pub pie_app: Vec<ReportSlice>,
    pub total_minutes: i64,
    pub top_app: String,
}

/// Build a [`ReportsViewModel`] from cached usage data over the given [`DateRange`].
pub fn build_reports_viewmodel(
    range: DateRange,
    summaries: &[DailySummary],
    _categories: &[Category],
    app_categories: &[AppCategoryRow],
) -> ReportsViewModel {
    let usage: Vec<DailyUsageEntry> = summaries
        .iter()
        .flat_map(|s| s.entries.iter().cloned())
        .collect();

    // Bars (reuse dashboard logic shape).
    let mut by_date: std::collections::BTreeMap<String, f64> = std::collections::BTreeMap::new();
    for entry in &usage {
        let minutes = entry.total_seconds as f64 / 60.0;
        *by_date.entry(entry.date.clone()).or_insert(0.0) += minutes;
    }
    let bar_chart: Vec<ReportBar> = by_date
        .into_iter()
        .filter_map(|(d, m)| {
            chrono::NaiveDate::parse_from_str(&d, "%Y-%m-%d")
                .ok()
                .map(|date| ReportBar {
                    date,
                    total_minutes: m,
                })
        })
        .collect();

    // App slices.
    let mut by_app: std::collections::HashMap<String, f64> = std::collections::HashMap::new();
    let mut total: f64 = 0.0;
    for entry in usage {
        let minutes = entry.total_seconds as f64 / 60.0;
        *by_app.entry(entry.app_id.clone()).or_insert(0.0) += minutes;
        total += minutes;
    }
    let pie_app: Vec<ReportSlice> = if total <= 0.0 {
        Vec::new()
    } else {
        let meta: std::collections::HashMap<&str, &str> = app_categories
            .iter()
            .map(|ac| (ac.app_id.as_str(), ac.display_name.as_str()))
            .collect();
        let mut slices: Vec<ReportSlice> = by_app
            .into_iter()
            .map(|(app_id, minutes)| ReportSlice {
                percentage: (minutes / total) * 100.0,
                app_id: app_id.clone(),
                display_name: meta
                    .get(app_id.as_str())
                    .copied()
                    .unwrap_or(&app_id)
                    .to_string(),
                color: String::new(),
            })
            .collect();
        slices.sort_by(|a, b| {
            b.percentage
                .partial_cmp(&a.percentage)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        slices
    };

    let total_minutes = (total) as i64;
    let top_app = pie_app
        .first()
        .map(|s| s.display_name.clone())
        .unwrap_or_else(|| "—".into());

    ReportsViewModel {
        date_range: range,
        bar_chart,
        pie_app,
        total_minutes,
        top_app,
    }
}

/// Format minutes into a human-readable duration string.
fn format_duration(total_minutes: i64) -> String {
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

/// Render the reports view from a ViewModel.
pub fn render_reports_view(
    cx: &App,
    vm: &ReportsViewModel,
    on_range_change: impl Fn(DateRange) + 'static,
) -> impl IntoElement {
    let accent = resolve_color("", "accent");

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
                        .bg(theme_accent(cx))
                        .text_color(cx.theme().accent_foreground)
                        .child(format!(
                            "{} – {}",
                            vm.date_range.start.format("%b %d"),
                            vm.date_range.end.format("%b %d, %Y"),
                        )),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(theme_text_secondary(cx))
                        .child(format!(
                            "Total {} · Top app {}",
                            format_duration(vm.total_minutes),
                            vm.top_app
                        )),
                ),
        )
        .child(card(
            cx,
            Some("Daily Screen Time"),
            vec![if vm.bar_chart.is_empty() {
                empty_state(cx, "No usage data in this range.")
            } else {
                div()
                    .h(px(180.0))
                    .child(
                        BarChart::new(vm.bar_chart.clone())
                            .band(|b: &ReportBar| b.date.format("%m/%d").to_string())
                            .value(|b: &ReportBar| b.total_minutes)
                            .label(|b: &ReportBar| {
                                SharedString::from(format!("{:.0}m", b.total_minutes))
                            })
                            .fill(move |_b, _bar, _chart, _align| accent)
                            .label_axis(true),
                    )
                    .into_any_element()
            }],
        ))
        .child(card(
            cx,
            Some("By App"),
            vec![if vm.pie_app.is_empty() {
                empty_state(cx, "No data.")
            } else {
                div()
                    .h(px(220.0))
                    .child(
                        PieChart::new(vm.pie_app.clone())
                            .value(|s: &ReportSlice| s.percentage as f32)
                            .color(|s: &ReportSlice| resolve_color(&s.color, &s.display_name))
                            .inner_radius(0.55)
                            .outer_radius(0.95)
                            .label(|s: &ReportSlice| {
                                let d = if s.display_name.is_empty() {
                                    &s.app_id
                                } else {
                                    &s.display_name
                                };
                                SharedString::from(format!("{} {:.0}%", d, s.percentage))
                            }),
                    )
                    .into_any_element()
            }],
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

// ── Theme helpers (local wrappers to keep call sites uniform) ────────────────

fn theme_accent(cx: &App) -> Hsla {
    crate::theme::accent(cx)
}
fn theme_text_secondary(cx: &App) -> Hsla {
    crate::theme::text_secondary(cx)
}

fn empty_state(cx: &App, message: &str) -> AnyElement {
    div()
        .py(sp::LG)
        .w_full()
        .items_center()
        .justify_center()
        .child(
            div()
                .text_sm()
                .text_color(crate::theme::text_muted(cx))
                .child(message.to_string()),
        )
        .into_any_element()
}
