//! Reusable UI primitives built on `gpui_component`.
//!
//! These wrap the component library's styling so every screen shares one
//! visual language: a `Card` panel, a `StatCard` KPI tile, and a `SectionTitle`.
//! Icons are rendered as theme-aware glyphs (no external font dependency).

use gpui::prelude::*;
use gpui::px;
use gpui::*;
use gpui_component::{button::Button, button::ButtonVariants, h_flex, v_flex};

use crate::theme::*;

/// Subtle elevation shadow applied to cards/panels.
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
        .child(body)
        .into_any_element()
}

/// A KPI tile: small label + large value, with an optional accent dot.
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
                .when(dot_el.is_some(), |el| el.child(dot_el.unwrap()))
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
        .text_color(text_secondary(cx))
        .child(title.to_string())
        .into_any_element()
}

/// A primary action button.
pub fn primary_button(id: impl Into<ElementId>, label: &str) -> Button {
    Button::new(id).label(label).primary()
}

/// A default/neutral button.
pub fn default_button(id: impl Into<ElementId>, label: &str) -> Button {
    Button::new(id).label(label)
}

/// A destructive button.
pub fn danger_button(id: impl Into<ElementId>, label: &str) -> Button {
    Button::new(id).label(label).danger()
}
