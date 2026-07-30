use gpui::prelude::*;
use gpui::px;
use gpui::*;
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::h_flex;

use crate::app::App as GuiApp;
use crate::components::{Card, List, ListState};
use crate::policies::{PolicyConfigForm, PolicyTarget};
use crate::theme::{self, rad, sp};

use super::PolListDelegate;

/// Header bar: "N policies configured" count + "New Policy" primary button.
pub struct PolicyHeader {
    pub count: usize,
    pub entity: Entity<GuiApp>,
}

impl RenderOnce for PolicyHeader {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let entity = self.entity.clone();
        h_flex()
            .justify_between()
            .items_center()
            .child(
                div()
                    .text_xs()
                    .text_color(theme::text_label(cx))
                    .child(format!("{} policies configured", self.count)),
            )
            .child(
                Button::new("new-policy")
                    .label("New Policy")
                    .primary()
                    .on_click(move |_, _window, app| {
                        entity.update(app, |this, cx2| {
                            this.policy_edit_id = None;
                            this.policy_edit =
                                Some((PolicyTarget::Any, PolicyConfigForm::default()));
                            cx2.notify();
                        });
                    }),
            )
    }
}

/// Card wrapping the virtualised policy list.
pub struct PolicyListCard {
    pub pol_list: Entity<ListState<PolListDelegate>>,
}

impl RenderOnce for PolicyListCard {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let card = Card {
            title: Some("Existing Policies".into()),
            children: vec![
                div()
                    .h(px(360.0))
                    .child(List::new(&self.pol_list))
                    .into_any_element(),
            ],
        };
        RenderOnce::render(card, window, cx)
    }
}

/// Card showing all configured categories with their color swatches.
pub struct CategoriesSection {
    pub categories: Vec<wellbeing_core::Category>,
}

impl RenderOnce for CategoriesSection {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let rows: Vec<AnyElement> = self
            .categories
            .iter()
            .map(|cat| {
                let color =
                    crate::theme::parse_hex(cat.color()).unwrap_or_else(|| theme::text_muted(cx));
                h_flex()
                    .gap_2()
                    .px(sp::MD)
                    .py(sp::SM)
                    .rounded(rad::md())
                    .child(div().size(px(12.0)).rounded(px(2.0)).bg(color))
                    .child(
                        div()
                            .text_sm()
                            .text_color(theme::text_primary(cx))
                            .child(cat.name()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme::text_muted(cx))
                            .child(cat.icon()),
                    )
                    .into_any_element()
            })
            .collect();

        let card = Card {
            title: Some("Categories".into()),
            children: if rows.is_empty() {
                vec![
                    div()
                        .py(sp::MD)
                        .text_sm()
                        .text_color(theme::text_muted(cx))
                        .child("No categories configured.")
                        .into_any_element(),
                ]
            } else {
                rows
            },
        };
        RenderOnce::render(card, window, cx)
    }
}
