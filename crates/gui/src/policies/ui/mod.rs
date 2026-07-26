//! Policies gpui rendering — all render logic, no D-Bus/data access.
//!
//! Implements `GuiApp` methods directly so callbacks can mutate the view's
//! editing state and persist via the daemon client.

mod editor;
mod list;

use gpui::prelude::*;
use gpui::px;
use gpui::*;
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::{h_flex, v_flex};

use crate::app::App as GuiApp;
use crate::components::card;
use crate::theme::{self, rad, sp};

use super::domain::{PoliciesViewModel, PolicyConfigForm, PolicyTarget};

#[cfg(feature = "gui-gpui")]
impl GuiApp {
    pub fn render_policies(
        &mut self,
        cx: &mut Context<Self>,
        vm: &PoliciesViewModel,
    ) -> AnyElement {
        let entity = cx.entity();

        v_flex()
            .gap_4()
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme::text_label(&*cx))
                            .child(format!("{} policies configured", vm.policies.len())),
                    )
                    .child(
                        Button::new("new-policy")
                            .label("New Policy")
                            .primary()
                            .on_click({
                                let entity = entity.clone();
                                move |_, _window, app| {
                                    entity.update(app, |this, cx2| {
                                        this.policy_edit_id = None;
                                        this.policy_edit = Some((
                                            PolicyTarget::App(String::new()),
                                            PolicyConfigForm::default(),
                                        ));
                                        cx2.notify();
                                    });
                                }
                            }),
                    ),
            )
            .child(self.render_policy_list(cx, vm, entity.clone()))
            .child(self.render_editor(cx, vm, entity.clone()))
            .child(self.render_categories(cx, vm, entity.clone()))
            .into_any_element()
    }

    fn render_categories(
        &self,
        cx: &mut Context<Self>,
        vm: &PoliciesViewModel,
        _entity: Entity<Self>,
    ) -> AnyElement {
        let rows: Vec<AnyElement> = vm
            .categories
            .iter()
            .map(|cat| {
                let color =
                    crate::theme::parse_hex(&cat.color).unwrap_or_else(|| theme::text_muted(&*cx));
                h_flex()
                    .gap_2()
                    .px(sp::MD)
                    .py(sp::SM)
                    .rounded(rad::md())
                    .child(div().size(px(12.0)).rounded(px(2.0)).bg(color))
                    .child(
                        div()
                            .text_sm()
                            .text_color(theme::text_primary(&*cx))
                            .child(cat.name.clone()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme::text_muted(&*cx))
                            .child(cat.icon.clone()),
                    )
                    .into_any_element()
            })
            .collect();

        card(
            &*cx,
            Some("Categories"),
            if rows.is_empty() {
                vec![empty_hint(&*cx, "No categories configured.")]
            } else {
                rows
            },
        )
    }
}

#[cfg(feature = "gui-gpui")]
fn empty_hint(cx: &App, message: &str) -> AnyElement {
    div()
        .py(sp::MD)
        .text_sm()
        .text_color(theme::text_muted(cx))
        .child(message.to_string())
        .into_any_element()
}

/// Stub returned when gpui is not enabled.
#[cfg(not(feature = "gui-gpui"))]
pub fn render_policies_view(_: &PoliciesViewModel) -> ! {
    panic!("gpui not enabled (feature gui-gpui is off)")
}
