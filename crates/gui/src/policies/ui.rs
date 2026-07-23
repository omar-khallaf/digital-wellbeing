//! Policies gpui rendering — all render logic, no D-Bus/data access.
//!
//! Implements `GuiApp` methods directly so callbacks can mutate the view's
//! editing state and persist via the daemon client.

use gpui::InteractiveElement;
use gpui::prelude::*;
use gpui::px;
use gpui::*;
use gpui_component::ActiveTheme;
use gpui_component::Disableable;
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::input::{Input, NumberInput};
use gpui_component::{h_flex, v_flex};

use crate::app::App as GuiApp;
use crate::components::card;
use crate::dbus::DaemonClient;
use crate::theme::{self, rad, sp};

use super::domain::{PoliciesViewModel, PolicyConfigForm, PolicyTarget, policy_input_from};

#[cfg(feature = "gui-gpui")]
impl GuiApp {
    /// Render the policies screen from the current ViewModel + live edit state.
    pub fn render_policies(
        &mut self,
        cx: &mut Context<Self>,
        vm: &PoliciesViewModel,
    ) -> AnyElement {
        let entity = cx.entity();
        let client = self
            .state
            .try_lock()
            .map(|s| s.client.clone())
            .unwrap_or_else(|_| panic!("daemon client unavailable (lock poisoned)"));

        v_flex()
            .gap_4()
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme::text_secondary(&*cx))
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
            .child(self.render_editor(cx, vm, entity.clone(), client))
            .child(self.render_categories(cx, vm, entity.clone()))
            .into_any_element()
    }

    /// Policy list — clicking a row loads it into the editor.
    fn render_policy_list(
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
                let is_selected = self.policy_edit_id.map(|id| id == p.id).unwrap_or(false);

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
                        let extra = p.extra_minutes;
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
                                        extra_minutes: extra,
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
                vec![empty_hint(&*cx, "No policies configured yet.")]
            } else {
                rows
            },
        )
    }

    /// The editor form (create/edit). Uses gpui_component Button controls;
    /// Save/Delete persist through the daemon client.
    fn render_editor(
        &self,
        cx: &mut Context<Self>,
        vm: &PoliciesViewModel,
        entity: Entity<Self>,
        client: DaemonClient,
    ) -> AnyElement {
        let (target, target_label, form) = match &self.policy_edit {
            Some((t, f)) => {
                let label = match t {
                    PolicyTarget::App(id) => {
                        if id.is_empty() {
                            "New policy \u{2014} pick an app".to_string()
                        } else {
                            format!("Editing app: {}", id)
                        }
                    }
                    PolicyTarget::Category(id) => {
                        let cat_name = vm
                            .categories
                            .iter()
                            .find(|c| c.id == wellbeing_core::CategoryId(*id))
                            .map(|c| c.name.as_str())
                            .unwrap_or("unknown");
                        format!("Editing category: {}", cat_name)
                    }
                };
                (t, label, f)
            }
            None => {
                return card(
                    &*cx,
                    Some("Policy Editor"),
                    vec![empty_hint(
                        &*cx,
                        "Select a policy to edit, or create a new one.",
                    )],
                );
            }
        };

        let show_time_limit = form.kind == "TimeLimit" || form.kind == "Notify";
        let hide_extra_time = form.kind == "Notify";
        let kinds = ["Block", "TimeLimit", "Notify"];
        let kind = form.kind.clone();
        let active = form.active;
        let is_app_target = matches!(target, PolicyTarget::App(_));

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
                                self.app_id_input
                                    .as_ref()
                                    .expect("app_id_input not initialized"),
                            )
                            .cleanable(true)
                            .flex_1(),
                        ),
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
                            .child("Extra time (min):"),
                    )
                    .child(
                        div().w(px(140.0)).child(
                            NumberInput::new(
                                self.extra_secs_input
                                    .as_ref()
                                    .expect("extra_secs_input not initialized"),
                            )
                            .appearance(true)
                            .disabled(false),
                        ),
                    )
                    .when(hide_extra_time, |el| el.opacity(0.4)),
            )
            .child(
                h_flex().gap_2().items_center().child(
                    Button::new("toggle-active")
                        .label(if active { "Enabled" } else { "Disabled" })
                        .when(active, |b| b.primary())
                        .on_click({
                            let entity = entity.clone();
                            move |_, _window, app| {
                                entity.update(app, |this, cx2| {
                                    if let Some((_, f)) = this.policy_edit.as_mut() {
                                        f.active = !f.active;
                                    }
                                    cx2.notify();
                                });
                            }
                        }),
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
                                let client = client.clone();
                                move |_, _window, app| {
                                    let (tl, es, ai) = {
                                        let me = entity.read(app);
                                        if me.policy_edit.is_none() {
                                            (0i64, 0i64, String::new())
                                        } else {
                                            let tl = me
                                                .time_limit_input
                                                .as_ref()
                                                .and_then(|e| {
                                                    e.read(app).value().parse::<i64>().ok()
                                                })
                                                .unwrap_or(0);
                                            let es = me
                                                .extra_secs_input
                                                .as_ref()
                                                .and_then(|e| {
                                                    e.read(app).value().parse::<i64>().ok()
                                                })
                                                .unwrap_or(0);
                                            let ai = me
                                                .app_id_input
                                                .as_ref()
                                                .map(|e| e.read(app).value().to_string())
                                                .unwrap_or_default();
                                            (tl, es, ai)
                                        }
                                    };
                                    let client = client.clone();
                                    entity.update(app, |this, cx2| {
                                        if let Some((_, ref mut form)) = this.policy_edit {
                                            form.time_limit_minutes = tl;
                                            form.extra_minutes = es;
                                            form.app_id = ai;
                                        }
                                        if let Some((target, form)) = this.policy_edit.clone() {
                                            let uid =
                                                this.state.try_lock().map(|s| s.uid).unwrap_or(0);
                                            let input = policy_input_from(target, &form, uid);
                                            let edit_id = this.policy_edit_id;
                                            let client = client.clone();
                                            let task = cx2.spawn(async move |this2, cx3| {
                                                let res = match edit_id {
                                                    Some(id) => {
                                                        client.update_policy(id, input).await
                                                    }
                                                    None => client
                                                        .create_policy(input)
                                                        .await
                                                        .map(|_| ()),
                                                };
                                                if res.is_ok() {
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
                    )
                    .child(
                        Button::new("delete-policy")
                            .label("Delete")
                            .danger()
                            .when(self.policy_edit_id.is_none(), |b| b.disabled(true))
                            .on_click({
                                let entity = entity.clone();
                                let client = client.clone();
                                move |_, _window, app| {
                                    let client = client.clone();
                                    entity.update(app, |this, cx2| {
                                        if let Some(id) = this.policy_edit_id {
                                            let client = client.clone();
                                            let task = cx2.spawn(async move |this2, cx3| {
                                                let _ = client.delete_policy(id).await;
                                                let _ = this2.update(cx3, |this3, cx4| {
                                                    this3.policy_edit = None;
                                                    this3.policy_edit_id = None;
                                                    cx4.notify();
                                                });
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

    /// Categories list with color swatches.
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
