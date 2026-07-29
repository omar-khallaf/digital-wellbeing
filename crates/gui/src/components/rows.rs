//! Row types and renderers for app and title list entries.
//! Shared by both the Dashboard and Reports screens.

use gpui::prelude::*;
use gpui::px;
use gpui::*;
use gpui_component::{h_flex, v_flex};

use crate::chart::empty_state;
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

/// Render a single app entry row.
pub fn render_app_entry_row(cx: &App, entry: &AppEntryView) -> AnyElement {
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
                .truncate()
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
}

/// Panel of app entry rows. Shows empty_state hint when no entries.
pub fn app_list_panel(cx: &App, entries: &[AppEntryView]) -> AnyElement {
    if entries.is_empty() {
        return empty_state(cx, "No usage data yet.").into_any_element();
    }
    v_flex()
        .gap_1()
        .children(entries.iter().map(|e| render_app_entry_row(cx, e)))
        .into_any_element()
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

/// Render a single title entry row.
pub fn render_title_entry_row(cx: &App, entry: &TitleEntryView) -> AnyElement {
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
                .overflow_hidden()
                .child(
                    div()
                        .text_sm()
                        .truncate()
                        .text_color(theme::text_primary(cx))
                        .child(entry.title.clone()),
                )
                .child(
                    div()
                        .text_xs()
                        .truncate()
                        .text_color(theme::text_muted(cx))
                        .child(entry.app_class.clone()),
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
}

/// Panel of title entry rows. Shows empty_state hint when no entries.
pub fn title_list_panel(cx: &App, entries: &[TitleEntryView]) -> AnyElement {
    if entries.is_empty() {
        return empty_state(cx, "No title usage data yet.").into_any_element();
    }
    v_flex()
        .gap_1()
        .children(entries.iter().map(|e| render_title_entry_row(cx, e)))
        .into_any_element()
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
