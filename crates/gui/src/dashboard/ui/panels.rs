//! Dashboard sub-panels — app list, title list, and block cards.

use chrono::Utc;
use gpui::prelude::*;
use gpui::*;
use gpui_component::scroll::ScrollableElement;
use gpui_component::{h_flex, v_flex};

use crate::chart::empty_state;
use crate::components::format_duration;
use crate::theme::{self, rad, sp};

use crate::dashboard::domain::{AppListEntry, BlockCardInfo, TitleListEntry};

pub(super) fn app_list_panel(cx: &App, entries: &[AppListEntry]) -> AnyElement {
    if entries.is_empty() {
        return empty_state(cx, "No usage data yet.").into_any_element();
    }

    let rows: Vec<AnyElement> = entries
        .iter()
        .map(|entry| {
            let blocked_color = if entry.is_blocked {
                theme::danger(cx)
            } else {
                theme::success(cx)
            };
            let dot_color = if entry.display_name.is_empty() {
                theme::color_from_str(&entry.app_id)
            } else {
                theme::color_from_str(&entry.display_name)
            };

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
                .child(div().size(px(10.0)).rounded(rad::full()).bg(dot_color))
                .child(
                    div()
                        .text_sm()
                        .flex_1()
                        .text_color(theme::text_primary(cx))
                        .child(entry.display_name.clone()),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(blocked_color)
                        .child(if entry.is_blocked { "BLOCKED" } else { "" }),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(theme::text_secondary(cx))
                        .child(format!("{:.1}%", entry.percentage)),
                )
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::BOLD)
                        .text_color(theme::text_primary(cx))
                        .child(format_duration(entry.total_millis)),
                )
                .into_any_element()
        })
        .collect();

    v_flex().gap_1().children(rows).into_any_element()
}

pub(super) fn title_list_panel(cx: &App, entries: &[TitleListEntry]) -> AnyElement {
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
                    div()
                        .text_xs()
                        .text_color(theme::text_primary(cx))
                        .min_w(px(64.0))
                        .child(entry.app_id.clone()),
                )
                .child(
                    div()
                        .text_sm()
                        .flex_1()
                        .text_color(theme::text_primary(cx))
                        .child(entry.title.clone()),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(theme::text_primary(cx))
                        .child(format!("{:.1}%", entry.percentage)),
                )
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::BOLD)
                        .text_color(theme::text_primary(cx))
                        .child(format_duration(entry.total_millis)),
                )
                .into_any_element()
        })
        .collect();

    div()
        .max_h(px(340.0))
        .overflow_scrollbar()
        .child(v_flex().gap_1().children(rows))
        .into_any_element()
}

pub(super) fn block_card(cx: &App, info: &BlockCardInfo) -> AnyElement {
    let now = Utc::now();
    let duration = now.signed_duration_since(info.blocked_since);
    let ago = if duration.num_minutes() < 1 {
        "just now".to_string()
    } else if duration.num_minutes() < 60 {
        format!("{} minutes ago", duration.num_minutes())
    } else if duration.num_hours() < 24 {
        format!("{} hours ago", duration.num_hours())
    } else {
        format!("{} days ago", duration.num_days())
    };

    let display = if info.display_name.is_empty() {
        &info.app_id
    } else {
        &info.display_name
    };

    h_flex()
        .gap_3()
        .items_center()
        .child(div().size(px(10.0)).rounded(rad::full()).bg(theme::danger(cx)))
        .child(
            v_flex()
                .gap_1()
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::BOLD)
                        .text_color(theme::text_primary(cx))
                        .child(format!("{} \u{2014} Blocked {}", display, ago)),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(theme::text_secondary(cx))
                        .child("Daily limit reached. Switch to the window and use the overlay controls to continue."),
                ),
        )
        .into_any_element()
}
