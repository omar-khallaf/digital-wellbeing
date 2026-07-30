//! Row types and renderers for app and title list entries.
//! Shared by both the Dashboard and Reports screens.

use gpui::prelude::*;
use gpui::px;
use gpui::{App, FontWeight, Hsla, Window, div};
use gpui_component::{h_flex, v_flex};

use crate::theme;
use crate::theme::*;

/// Optional badge rendered after the display name (e.g. "BLOCKED").
#[derive(Debug, Clone)]
pub struct AppBadge {
    pub text: String,
    pub color: Hsla,
}

/// Shared data for a single app entry row — used by both the dashboard
/// and reports "all apps" panels.
#[derive(Debug, Clone)]
pub struct AppEntryView {
    pub rank: usize,
    pub display_name: String,
    pub total_millis: i64,
    pub percentage: f64,
    pub dot_color: Option<Hsla>,
    pub badge: Option<AppBadge>,
}

pub struct AppEntryRow {
    pub entry: AppEntryView,
}

impl RenderOnce for AppEntryRow {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
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
                    .child(format!("#{}", self.entry.rank)),
            )
            .when_some(self.entry.dot_color, |el, color| {
                el.child(div().size(px(10.0)).rounded(rad::full()).bg(color))
            })
            .child(
                div()
                    .text_sm()
                    .flex_1()
                    .truncate()
                    .text_color(theme::text_primary(cx))
                    .child(self.entry.display_name),
            )
            .when_some(self.entry.badge, |el, badge| {
                el.child(div().text_xs().text_color(badge.color).child(badge.text))
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
                            .child(format_duration(self.entry.total_millis)),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme::text_label(cx))
                            .child(format!("{:.1}%", self.entry.percentage)),
                    ),
            )
    }
}

pub struct AppListPanel {
    pub entries: Vec<AppEntryView>,
}

impl RenderOnce for AppListPanel {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        if self.entries.is_empty() {
            return div()
                .text_sm()
                .text_color(theme::text_muted(cx))
                .child("No usage data yet.")
                .into_any_element();
        }
        v_flex()
            .gap_1()
            .children(self.entries.into_iter().map(|e| {
                AppEntryRow { entry: e }
                    .render(window, cx)
                    .into_any_element()
            }))
            .into_any_element()
    }
}

/// Shared data for a single title entry row.
#[derive(Debug, Clone)]
pub struct TitleEntryView {
    pub rank: usize,
    pub app_class: String,
    pub title: String,
    pub total_millis: i64,
    pub percentage: f64,
}

pub struct TitleEntryRow {
    pub entry: TitleEntryView,
}

impl RenderOnce for TitleEntryRow {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
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
                    .child(format!("#{}", self.entry.rank)),
            )
            .child(
                v_flex()
                    .flex_1()
                    .gap_0()
                    .overflow_hidden()
                    .child(
                        div()
                            .text_sm()
                            .truncate()
                            .text_color(theme::text_primary(cx))
                            .child(self.entry.title),
                    )
                    .child(
                        div()
                            .text_xs()
                            .truncate()
                            .text_color(theme::text_muted(cx))
                            .child(self.entry.app_class),
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
                            .child(format_duration(self.entry.total_millis)),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme::text_label(cx))
                            .child(format!("{:.1}%", self.entry.percentage)),
                    ),
            )
    }
}

pub struct TitleListPanel {
    pub entries: Vec<TitleEntryView>,
}

impl RenderOnce for TitleListPanel {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        if self.entries.is_empty() {
            return div()
                .text_sm()
                .text_color(theme::text_muted(cx))
                .child("No title usage data yet.")
                .into_any_element();
        }
        v_flex()
            .gap_1()
            .children(self.entries.into_iter().map(|e| {
                TitleEntryRow { entry: e }
                    .render(window, cx)
                    .into_any_element()
            }))
            .into_any_element()
    }
}

/// Format milliseconds to a human-readable duration string.
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
