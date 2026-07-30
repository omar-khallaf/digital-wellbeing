use gpui::prelude::*;
use gpui::*;
use gpui_component::Disableable;
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::h_flex;

use wellbeing_core::AppClass;

use crate::app::{App as GuiApp, RenderMode};
use crate::policies::domain::{PoliciesViewModel, policy_input_from};

#[cfg(feature = "gui-gpui")]
pub(crate) struct SaveDeleteButtons {
    pub entity: Entity<GuiApp>,
    pub policy_edit_id: Option<wellbeing_core::PolicyId>,
}

#[cfg(feature = "gui-gpui")]
impl RenderOnce for SaveDeleteButtons {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let entity = self.entity;
        let policy_edit_id = self.policy_edit_id;
        h_flex()
            .gap_2()
            .child(
                Button::new("save-policy")
                    .label("Save")
                    .primary()
                    .on_click({
                        let entity = entity.clone();
                        move |_, _window, app| {
                            let (tl, ai, pr) = {
                                let me = entity.read(app);
                                if me.policy_edit.is_none() {
                                    (0i64, String::new(), 100i64)
                                } else {
                                    let tl = me
                                        .time_limit_input
                                        .as_ref()
                                        .and_then(|e| e.read(app).value().parse::<i64>().ok())
                                        .unwrap_or(0);
                                    let ai = me
                                        .app_class_input
                                        .as_ref()
                                        .map(|e| e.read(app).value().to_string())
                                        .unwrap_or_default();
                                    let pr = me
                                        .priority_input
                                        .as_ref()
                                        .and_then(|e| e.read(app).value().parse::<i64>().ok())
                                        .unwrap_or(100);
                                    (tl, ai, pr)
                                }
                            };
                            let repo = entity.read(app).policies_repo.clone();
                            let Some(repo) = repo else {
                                return;
                            };
                            let is_admin = entity.read(app).state.mode == RenderMode::Admin;
                            entity.update(app, |this, cx2| {
                                if let Some((_, ref mut form)) = this.policy_edit {
                                    form.time_limit_minutes = tl;
                                    form.app_class = AppClass::new(&ai)
                                        .unwrap_or_else(|_| form.app_class.clone());
                                    form.priority = pr;
                                    form.schedule_json = serde_json::to_string(&form.schedules)
                                        .unwrap_or_else(|_| "[]".into());
                                }
                                if let Some((target, form)) = this.policy_edit.clone() {
                                    let uid = this.state.uid;
                                    let input = policy_input_from(target, &form, uid);
                                    let edit_id = this.policy_edit_id;
                                    let task = cx2.spawn(async move |this2, cx3| {
                                        let res = match edit_id {
                                            Some(id) => repo.update_policy(id, input).await,
                                            None => repo.create_policy(input).await.map(|_| ()),
                                        };
                                        if res.is_ok() {
                                            if let Ok(data) = repo.fetch_all(uid).await {
                                                let mut vm = PoliciesViewModel {
                                                    data: Some(data),
                                                    is_admin,
                                                    ..Default::default()
                                                };
                                                vm.recompute_derived();
                                                let _ = this2.update(cx3, |this3, cx4| {
                                                    this3.policies_vm = Some(vm);
                                                    this3.policy_edit = None;
                                                    this3.policy_edit_id = None;
                                                    cx4.notify();
                                                });
                                            } else {
                                                let _ = this2.update(cx3, |this3, cx4| {
                                                    this3.policy_edit = None;
                                                    this3.policy_edit_id = None;
                                                    cx4.notify();
                                                });
                                            }
                                        }
                                    });
                                    this.set_policy_task(task);
                                }
                            });
                        }
                    }),
            )
            .child(
                Button::new("delete-policy")
                    .label("Delete")
                    .danger()
                    .when(policy_edit_id.is_none(), |b| b.disabled(true))
                    .on_click({
                        let entity = entity.clone();
                        move |_, _window, app| {
                            let repo = entity.read(app).policies_repo.clone();
                            let Some(repo) = repo else {
                                return;
                            };
                            let is_admin = entity.read(app).state.mode == RenderMode::Admin;
                            let uid = entity.read(app).state.uid;
                            entity.update(app, |this, cx2| {
                                if let Some(id) = this.policy_edit_id {
                                    let task = cx2.spawn(async move |this2, cx3| {
                                        let _ = repo.delete_policy(id).await;
                                        if let Ok(data) = repo.fetch_all(uid).await {
                                            let mut vm = PoliciesViewModel {
                                                data: Some(data),
                                                is_admin,
                                                ..Default::default()
                                            };
                                            vm.recompute_derived();
                                            let _ = this2.update(cx3, |this3, cx4| {
                                                this3.policies_vm = Some(vm);
                                                this3.policy_edit = None;
                                                this3.policy_edit_id = None;
                                                cx4.notify();
                                            });
                                        } else {
                                            let _ = this2.update(cx3, |this3, cx4| {
                                                this3.policy_edit = None;
                                                this3.policy_edit_id = None;
                                                cx4.notify();
                                            });
                                        }
                                    });
                                    this.set_policy_task(task);
                                }
                            });
                        }
                    }),
            )
            .into_any_element()
    }
}
