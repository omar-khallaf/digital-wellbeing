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
use crate::components::{CategoriesSection, PolicyHeader, PolicyListCard};
use crate::theme::{self, sp};

use super::domain::PoliciesViewModel;

#[cfg(feature = "gui-gpui")]
impl GuiApp {
    /// Render the full policies screen composing reusable components.
    /// The editor is kept as a method because it reads 7+ `self.*` input
    /// entities and the live `policy_edit` state.
    pub fn render_policies(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        vm: &PoliciesViewModel,
    ) -> AnyElement {
        let entity = cx.entity();
        let loaded = self.pol_list.is_some();

        v_flex()
            .gap_4()
            .child(
                RenderOnce::render(
                    PolicyHeader {
                        count: vm.policies.len(),
                        entity: entity.clone(),
                    },
                    window,
                    cx,
                )
                .into_any_element(),
            )
            .child(
                self.pol_list
                    .as_ref()
                    .map(|pol_list| {
                        RenderOnce::render(
                            PolicyListCard {
                                pol_list: pol_list.clone(),
                            },
                            window,
                            cx,
                        )
                        .into_any_element()
                    })
                    .unwrap_or_else(|| {
                        div()
                            .py(sp::MD)
                            .text_sm()
                            .text_color(theme::text_muted(&*cx))
                            .child(if loaded { "Loading..." } else { "" })
                            .into_any_element()
                    }),
            )
            .child(self.render_editor(window, cx, vm, entity.clone()))
            .child(
                RenderOnce::render(
                    CategoriesSection {
                        categories: vm.categories.clone(),
                    },
                    window,
                    cx,
                )
                .into_any_element(),
            )
            .into_any_element()
    }
}

/// Small hint text used by the editor when no policy is selected.
#[cfg(feature = "gui-gpui")]
pub struct EmptyHint {
    pub message: SharedString,
}

#[cfg(feature = "gui-gpui")]
impl RenderOnce for EmptyHint {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        div()
            .py(sp::MD)
            .text_sm()
            .text_color(theme::text_muted(cx))
            .child(self.message)
            .into_any_element()
    }
}

/// Render the complete policies view.
/// Takes `this: &mut Self` to avoid re-entrant `entity.update()` during
/// `Render::render()` — callbacks use `cx.entity()` for deferred mutations.
#[cfg(feature = "gui-gpui")]
pub fn render_policies_view(
    this: &mut GuiApp,
    window: &mut Window,
    cx: &mut Context<GuiApp>,
    vm: &PoliciesViewModel,
) -> AnyElement {
    this.render_policies(window, cx, vm)
}

/// Stub — gpui types unavailable without feature.
#[cfg(not(feature = "gui-gpui"))]
pub fn render_policies_view(_: &mut (), _: &mut (), _: &mut (), _: &PoliciesViewModel) -> ! {
    panic!("gpui not enabled (feature gui-gpui is off)")
}
