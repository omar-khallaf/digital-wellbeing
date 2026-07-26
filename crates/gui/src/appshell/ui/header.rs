use gpui::prelude::*;
use gpui::px;
use gpui::*;
use gpui_component::ActiveTheme;
use gpui_component::h_flex;

use crate::appshell::domain::Tab;
use crate::theme::*;

pub fn header(cx: &gpui::App, active: Tab, mode: &str) -> AnyElement {
    h_flex()
        .h(px(56.0))
        .px(sp::LG)
        .bg(surface(cx))
        .border_b_1()
        .border_color(border(cx))
        .justify_between()
        .items_center()
        .child(
            h_flex()
                .gap_3()
                .items_center()
                .child(
                    div()
                        .text_lg()
                        .font_weight(FontWeight::BOLD)
                        .text_color(text_primary(cx))
                        .child(active.label()),
                )
                .child(
                    div()
                        .text_xs()
                        .px(sp::XS)
                        .py(px(1.0))
                        .rounded(rad::sm())
                        .bg(accent(cx))
                        .text_color(cx.theme().accent_foreground)
                        .child(format!("v{}", env!("CARGO_PKG_VERSION"))),
                ),
        )
        .child(
            h_flex().gap_2().items_center().child(
                div()
                    .text_xs()
                    .text_color(text_label(cx))
                    .child(format!("{} session", mode)),
            ),
        )
        .into_any_element()
}
