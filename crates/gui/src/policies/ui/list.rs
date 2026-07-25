use gpui::InteractiveElement;
use gpui::prelude::*;
use gpui::*;
use gpui_component::ActiveTheme;

use crate::app::App as GuiApp;
use crate::components::card;
use crate::theme::{self, rad, sp};

use crate::policies::domain::{PoliciesViewModel, PolicyConfigForm, PolicyTarget};

#[cfg(feature = "gui-gpui")]
impl GuiApp {
    pub fn render_policy_list(
        &self,
        cx: &mut Context<Self>,
        vm: &PoliciesViewModel,
        entity: Entity<Self>,
    ) -> AnyElement {
        let rows: Vec<AnyElement> = vm
            .policies
            .iter()
            .map(|p| {
                let kind_display = match p.action {
                    wellbeing_core::PolicyKind::Block => "Block",
                    wellbeing_core::PolicyKind::TimeLimit => "Time Limit",
                    wellbeing_core::PolicyKind::Notify => "Notify",
                };
                let target = if !p.app_id.is_empty() {
                    format!("App: {}", p.app_id)
                } else if p.category_id > 0 {
                    format!("Category: {}", p.category_id)
                } else {
                    "All".to_string()
                };
                let status = if p.active { "Active" } else { "Inactive" };
                let is_selected = self.policy_edit_id == Some(p.id);

                div()
                    .id(format!("policy-row-{}", p.id.0))
                    .gap_3()
                    .px(sp::MD)
                    .py(sp::SM)
                    .rounded(rad::md())
                    .cursor_pointer()
                    .when(is_selected, |el| el.bg(cx.theme().accent))
                    .when(!is_selected, |el| el.hover(|s| s.bg(cx.theme().border)))
                    .on_click({
                        let entity = entity.clone();
                        let pid = p.id;
                        let kind = p.action;
                        let app_id = p.app_id.clone();
                        let cat_id = p.category_id;
                        let tls = p.time_limit_minutes;
                        let schedule = p.schedule_json.clone();
                        let active = p.active;
                        move |_, _window, app| {
                            entity.update(app, |this, cx2| {
                                this.policy_edit_id = Some(pid);
                                this.policy_edit = Some((
                                    if cat_id > 0 {
                                        PolicyTarget::Category(cat_id)
                                    } else {
                                        PolicyTarget::App(app_id.clone())
                                    },
                                    PolicyConfigForm {
                                        kind: match kind {
                                            wellbeing_core::PolicyKind::Block => "Block".into(),
                                            wellbeing_core::PolicyKind::TimeLimit => {
                                                "TimeLimit".into()
                                            }
                                            wellbeing_core::PolicyKind::Notify => "Notify".into(),
                                        },
                                        time_limit_minutes: tls,
                                        schedule_json: schedule.clone(),
                                        active,
                                        app_id: app_id.clone(),
                                    },
                                ));
                                cx2.notify();
                            });
                        }
                    })
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme::text_primary(&*cx))
                            .child(kind_display.to_string()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme::text_secondary(&*cx))
                            .flex_1()
                            .child(target),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(if p.active {
                                theme::success(&*cx)
                            } else {
                                theme::text_muted(&*cx)
                            })
                            .child(status.to_string()),
                    )
                    .into_any_element()
            })
            .collect();

        card(
            &*cx,
            Some("Existing Policies"),
            if rows.is_empty() {
                vec![super::empty_hint(&*cx, "No policies configured yet.")]
            } else {
                rows
            },
        )
    }
}
