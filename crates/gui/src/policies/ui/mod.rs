//! Policies gpui rendering — composes reusable components from
//! `crate::components::*` and keeps policy-specific editor logic here.
//!
//! Follows the free-function pattern: `render_policies_view(this, cx, vm)`
//! is the single entry point, composing `policy_header`, `policy_list_card`,
//! `categories_section`, and the policy-internal `render_editor`.

mod editor;

use gpui::prelude::*;
use gpui::*;
use gpui_component::v_flex;

use crate::app::App as GuiApp;
use crate::components::{categories_section, policy_header, policy_list_card};
use crate::theme::{self, sp};

use super::domain::PoliciesViewModel;

#[cfg(feature = "gui-gpui")]
impl GuiApp {
    /// Render the full policies screen composing reusable components.
    /// The editor is kept as a method because it reads 7+ `self.*` input
    /// entities and the live `policy_edit` state.
    pub fn render_policies(
        &mut self,
        cx: &mut Context<Self>,
        vm: &PoliciesViewModel,
    ) -> AnyElement {
        let entity = cx.entity();
        let loaded = self.pol_list.is_some();

        v_flex()
            .gap_4()
            .child(policy_header(&*cx, vm.policies.len(), entity.clone()))
            .child(
                self.pol_list
                    .as_ref()
                    .map(|pol_list| policy_list_card(&*cx, pol_list))
                    .unwrap_or_else(|| {
                        div()
                            .py(sp::MD)
                            .text_sm()
                            .text_color(theme::text_muted(&*cx))
                            .child(if loaded { "Loading..." } else { "" })
                            .into_any_element()
                    }),
            )
            .child(self.render_editor(cx, vm, entity.clone()))
            .child(categories_section(&*cx, &vm.categories))
            .into_any_element()
    }
}

/// Small hint text used by the editor when no policy is selected.
#[cfg(feature = "gui-gpui")]
pub(crate) fn empty_hint(cx: &App, message: &str) -> AnyElement {
    div()
        .py(sp::MD)
        .text_sm()
        .text_color(theme::text_muted(cx))
        .child(message.to_string())
        .into_any_element()
}

/// Render the complete policies view.
/// Takes `this: &mut Self` to avoid re-entrant `entity.update()` during
/// `Render::render()` — callbacks use `cx.entity()` for deferred mutations.
#[cfg(feature = "gui-gpui")]
pub fn render_policies_view(
    this: &mut GuiApp,
    cx: &mut Context<GuiApp>,
    vm: &PoliciesViewModel,
) -> AnyElement {
    this.render_policies(cx, vm)
}

/// Stub — gpui types unavailable without feature.
#[cfg(not(feature = "gui-gpui"))]
pub fn render_policies_view(_: &mut (), _: &mut (), _: &PoliciesViewModel) -> ! {
    panic!("gpui not enabled (feature gui-gpui is off)")
}
