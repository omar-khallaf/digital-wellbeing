use gpui::prelude::*;
use gpui::*;
use gpui_component::v_flex;

use crate::app::App as GuiApp;
use crate::components::Card;
use crate::theme;

use crate::policies::domain::{PoliciesViewModel, PolicyTarget};
use crate::policies::ui::EmptyHint;

#[cfg(feature = "gui-gpui")]
mod actions;
#[cfg(feature = "gui-gpui")]
mod form_body;
#[cfg(feature = "gui-gpui")]
mod schedule;

#[cfg(feature = "gui-gpui")]
use actions::SaveDeleteButtons;
#[cfg(feature = "gui-gpui")]
use form_body::FormBody;
#[cfg(feature = "gui-gpui")]
use schedule::ScheduleSection;

#[cfg(feature = "gui-gpui")]
impl GuiApp {
    /// The editor form (create/edit). Uses gpui_component Button controls;
    /// Save/Delete persist through the daemon client.
    pub fn render_editor(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
        vm: &PoliciesViewModel,
        entity: Entity<Self>,
    ) -> AnyElement {
        let (target, target_label, form) = match &self.policy_edit {
            Some((t, f)) => {
                let label = match t {
                    PolicyTarget::App(id) => {
                        format!("Editing app: {}", id)
                    }
                    PolicyTarget::Category(name) => {
                        format!("Editing category: {}", name)
                    }
                    PolicyTarget::Domain(d) => {
                        format!("Editing domain: {}", d)
                    }
                    PolicyTarget::Any => "Editing: All targets".to_string(),
                };
                (t, label, f)
            }
            None => {
                return RenderOnce::render(
                    Card {
                        title: Some("Policy Editor".into()),
                        children: vec![
                            RenderOnce::render(
                                EmptyHint {
                                    message: "Select a policy to edit, or create a new one.".into(),
                                },
                                window,
                                cx,
                            )
                            .into_any_element(),
                        ],
                    },
                    window,
                    cx,
                )
                .into_any_element();
            }
        };

        let show_time_limit = form.kind == "TimeLimit" || form.kind == "Notify";
        let is_app_target = matches!(target, PolicyTarget::App(_));
        let is_cat_target = matches!(target, PolicyTarget::Category(_));

        let editor = v_flex()
            .gap_3()
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme::text_primary(&*cx))
                    .child(target_label),
            )
            .child(RenderOnce::render(
                FormBody {
                    entity: entity.clone(),
                    form: form.clone(),
                    vm: vm.clone(),
                    target: target.clone(),
                    is_app_target,
                    is_cat_target,
                    show_time_limit,
                    app_class_input: self
                        .app_class_input
                        .clone()
                        .expect("app_class_input not initialized"),
                    time_limit_input: self
                        .time_limit_input
                        .clone()
                        .expect("time_limit_input not initialized"),
                    priority_input: self
                        .priority_input
                        .clone()
                        .expect("priority_input not initialized"),
                },
                window,
                cx,
            ))
            .child(RenderOnce::render(
                ScheduleSection {
                    entity: entity.clone(),
                    form: form.clone(),
                    schedule_start_hour: self
                        .schedule_start_hour
                        .clone()
                        .expect("schedule inputs not initialized"),
                    schedule_start_minute: self
                        .schedule_start_minute
                        .clone()
                        .expect("schedule inputs not initialized"),
                    schedule_end_hour: self
                        .schedule_end_hour
                        .clone()
                        .expect("schedule inputs not initialized"),
                    schedule_end_minute: self
                        .schedule_end_minute
                        .clone()
                        .expect("schedule inputs not initialized"),
                },
                window,
                cx,
            ))
            .child(RenderOnce::render(
                SaveDeleteButtons {
                    entity: entity.clone(),
                    policy_edit_id: self.policy_edit_id,
                },
                window,
                cx,
            ));

        RenderOnce::render(
            Card {
                title: Some("Policy Editor".into()),
                children: vec![editor.into_any_element()],
            },
            window,
            cx,
        )
        .into_any_element()
    }
}
