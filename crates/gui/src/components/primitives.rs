//! Primitive components — card, stat_card, section_title.
//! Every screen uses these; they are the lowest-level visual building blocks.

use gpui::SharedString;
use gpui::prelude::*;
use gpui::px;
use gpui::{AnyElement, App, BoxShadow, FontWeight, Hsla, Window, div, hsla};
use gpui_component::{h_flex, v_flex};

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

/// Padded card panel with optional title header.
/// `title` = `None` renders a bordered container (good for chart holders).
pub struct Card {
    pub title: Option<SharedString>,
    pub children: Vec<AnyElement>,
}

impl RenderOnce for Card {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let body = match self.title {
            Some(t) => v_flex()
                .gap_2()
                .child(SectionTitle { title: t }.render(window, cx))
                .children(self.children)
                .into_any_element(),
            None => v_flex().gap_2().children(self.children).into_any_element(),
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
    }
}

/// KPI stat card — value + label with optional colored dot.
pub struct StatCard {
    pub value: SharedString,
    pub label: SharedString,
    pub dot_color: Option<Hsla>,
}

impl RenderOnce for StatCard {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
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
                    .when_some(self.dot_color, |el, c| {
                        el.child(div().size(px(8.0)).rounded(rad::full()).bg(c))
                    })
                    .child(
                        div()
                            .text_xs()
                            .text_color(text_muted(cx))
                            .child(self.label.to_string()),
                    ),
            )
            .child(
                div()
                    .mt_1()
                    .text_2xl()
                    .font_weight(FontWeight::BOLD)
                    .text_color(text_primary(cx))
                    .child(self.value.to_string()),
            )
    }
}

/// Section title text used as card headers.
pub struct SectionTitle {
    pub title: SharedString,
}

impl RenderOnce for SectionTitle {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        div()
            .text_sm()
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(text_primary(cx))
            .child(self.title.to_string())
    }
}
