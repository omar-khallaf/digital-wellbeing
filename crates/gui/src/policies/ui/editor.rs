use gpui::prelude::*;
use gpui::px;
use gpui::*;
use gpui_component::ActiveTheme;
use gpui_component::Disableable;
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::input::{Input, NumberInput};
use gpui_component::{h_flex, v_flex};

use wellbeing_core::AppClass;

use crate::app::{App as GuiApp, RenderMode};
use crate::components::card;
use crate::policies::build_policies_viewmodel;
use crate::theme;
use crate::theme::{rad, sp};

use crate::policies::domain::{PoliciesViewModel, PolicyTarget, policy_input_from};

#[cfg(feature = "gui-gpui")]
impl GuiApp {
    /// The editor form (create/edit). Uses gpui_component Button controls;
    /// Save/Delete persist through the daemon client.
    pub fn render_editor(
        &self,
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
                return card(
                    &*cx,
                    Some("Policy Editor"),
                    vec![super::empty_hint(
                        &*cx,
                        "Select a policy to edit, or create a new one.",
                    )],
                );
            }
        };

        let show_time_limit = form.kind == "TimeLimit" || form.kind == "Notify";
        let kinds = ["Allow", "Block", "TimeLimit", "Notify"];
        let kind = &form.kind;

        let target_types: [(&str, &str); 3] =
            [("App", "App"), ("Category", "Category"), ("Any", "Any")];
        let current_target_label = match target {
            PolicyTarget::App(_) => "App",
            PolicyTarget::Category(_) => "Category",
            PolicyTarget::Domain(_) => "Domain",
            PolicyTarget::Any => "Any",
        };
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
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme::text_primary(&*cx))
                            .child("Target:"),
                    )
                    .children(target_types.iter().map(|(key, label)| {
                        let is_active = current_target_label == *key;
                        let key_str = key.to_string();
                        let entity = entity.clone();
                        Button::new(format!("target-{}", key))
                            .label(*label)
                            .when(is_active, |b| b.primary())
                            .on_click(move |_, _window, app| {
                                entity.update(app, |this, cx2| {
                                    if let Some((ref mut t, ref mut f)) = this.policy_edit {
                                        match key_str.as_str() {
                                            "Category" => {
                                                *t =
                                                    PolicyTarget::Category(f.category_name.clone());
                                            }
                                            "Any" => {
                                                *t = PolicyTarget::Any;
                                            }
                                            _ => {
                                                *t = PolicyTarget::App(f.app_class.clone());
                                            }
                                        }
                                    }
                                    cx2.notify();
                                });
                            })
                    })),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme::text_primary(&*cx))
                            .child("Kind:"),
                    )
                    .children(kinds.iter().map(|k| {
                        let is_active = kind == *k;
                        Button::new(format!("kind-{}", k))
                            .label(*k)
                            .when(is_active, |b| b.primary())
                            .on_click({
                                let entity = entity.clone();
                                let kind_str = (*k).to_string();
                                move |_, _window, app| {
                                    entity.update(app, |this, cx2| {
                                        if let Some((_, f)) = this.policy_edit.as_mut() {
                                            f.kind = kind_str.clone();
                                        }
                                        cx2.notify();
                                    });
                                }
                            })
                    })),
            )
            .when(is_app_target, |el| {
                el.child(
                    h_flex()
                        .w_full()
                        .gap_2()
                        .items_center()
                        .child(
                            div()
                                .text_xs()
                                .text_color(theme::text_primary(&*cx))
                                .child("App ID (window class):"),
                        )
                        .child(
                            Input::new(
                                self.app_class_input
                                    .as_ref()
                                    .expect("app_class_input not initialized"),
                            )
                            .cleanable(true)
                            .flex_1(),
                        ),
                )
            })
            .when(is_cat_target, |el| {
                let cat_rows: Vec<AnyElement> = vm
                    .categories
                    .iter()
                    .map(|cat| {
                        let is_sel = form.category_name == cat.name;
                        let color = crate::theme::parse_hex(&cat.color)
                            .unwrap_or_else(|| theme::text_muted(&*cx));
                        let cat_name = cat.name.clone();
                        let entity = entity.clone();
                        div()
                            .id(format!("cat-opt-{}", cat.id.0))
                            .gap_2()
                            .px(sp::MD)
                            .py(sp::SM)
                            .rounded(rad::md())
                            .cursor_pointer()
                            .when(is_sel, |el| el.bg(cx.theme().accent))
                            .when(!is_sel, |el| el.hover(|s| s.bg(cx.theme().border)))
                            .on_click(move |_, _window, app| {
                                entity.update(app, |this, cx2| {
                                    if let Some((ref mut t, ref mut f)) = this.policy_edit {
                                        f.category_name = cat_name.clone();
                                        *t = PolicyTarget::Category(f.category_name.clone());
                                    }
                                    cx2.notify();
                                });
                            })
                            .child(
                                h_flex()
                                    .gap_2()
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
                                    ),
                            )
                            .into_any_element()
                    })
                    .collect();

                let cat_section = if cat_rows.is_empty() {
                    super::empty_hint(&*cx, "No categories available.")
                } else {
                    v_flex().gap_1().children(cat_rows).into_any_element()
                };

                el.child(
                    v_flex()
                        .gap_1()
                        .child(
                            div()
                                .text_xs()
                                .text_color(theme::text_primary(&*cx))
                                .child("Category:"),
                        )
                        .child(cat_section),
                )
            })
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme::text_primary(&*cx))
                            .child("Time limit (min):"),
                    )
                    .child(
                        div().w(px(140.0)).child(
                            NumberInput::new(
                                self.time_limit_input
                                    .as_ref()
                                    .expect("time_limit_input not initialized"),
                            )
                            .appearance(true)
                            .disabled(false),
                        ),
                    )
                    .when(!show_time_limit, |el| el.opacity(0.4)),
            )
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme::text_primary(&*cx))
                            .child("Priority (lower = first):"),
                    )
                    .child(
                        div().w(px(140.0)).child(
                            NumberInput::new(
                                self.priority_input
                                    .as_ref()
                                    .expect("priority_input not initialized"),
                            )
                            .appearance(true)
                            .disabled(false),
                        ),
                    ),
            )
            .child(
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
                                                .and_then(|e| {
                                                    e.read(app).value().parse::<i64>().ok()
                                                })
                                                .unwrap_or(0);
                                            let ai = me
                                                .app_class_input
                                                .as_ref()
                                                .map(|e| e.read(app).value().to_string())
                                                .unwrap_or_default();
                                            let pr = me
                                                .priority_input
                                                .as_ref()
                                                .and_then(|e| {
                                                    e.read(app).value().parse::<i64>().ok()
                                                })
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
                                        }
                                        if let Some((target, form)) = this.policy_edit.clone() {
                                            let uid = this.state.uid;
                                            let input = policy_input_from(target, &form, uid);
                                            let edit_id = this.policy_edit_id;
                                            let task = cx2.spawn(async move |this2, cx3| {
                                                let res = match edit_id {
                                                    Some(id) => repo.update_policy(id, input).await,
                                                    None => {
                                                        repo.create_policy(input).await.map(|_| ())
                                                    }
                                                };
                                                if res.is_ok() {
                                                    if let Ok(data) = repo.fetch_all(uid).await {
                                                        let app_classs: Vec<AppClass> = data
                                                            .app_list
                                                            .iter()
                                                            .map(|ac| ac.app_class.clone())
                                                            .collect();
                                                        let vm = build_policies_viewmodel(
                                                            &data.policies,
                                                            &data.categories,
                                                            &app_classs,
                                                            is_admin,
                                                        );
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
                                        cx2.notify();
                                    });
                                }
                            }),
                    )
                    .child(
                        Button::new("delete-policy")
                            .label("Delete")
                            .danger()
                            .when(self.policy_edit_id.is_none(), |b| b.disabled(true))
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
                                                    let app_classs: Vec<AppClass> = data
                                                        .app_list
                                                        .iter()
                                                        .map(|ac| ac.app_class.clone())
                                                        .collect();
                                                    let vm = build_policies_viewmodel(
                                                        &data.policies,
                                                        &data.categories,
                                                        &app_classs,
                                                        is_admin,
                                                    );
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
                                        cx2.notify();
                                    });
                                }
                            }),
                    ),
            );

        card(&*cx, Some("Policy Editor"), vec![editor.into_any_element()])
    }
}
