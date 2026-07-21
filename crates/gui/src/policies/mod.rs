//! Policies management screen — ViewModel, form types, and gpui components.
//!
//! # Data flow
//! 1. `build_policies_viewmodel()` transforms raw `Policy`/`Category`/app_id
//!    slices into a `PoliciesViewModel`.
//! 2. `App::render_policies()` (in `app.rs`) consumes the ViewModel and the
//!    view's live edit state, producing the interactive gpui element tree.
//!
//! # Feature gate
//! The gpui-dependent render functions are only compiled when the `gui-gpui`
//! feature is active (default on).  Data types and the builder fn are
//! unconditional so the daemon crate can reference them.

use gpui::InteractiveElement;
use gpui::prelude::*;
use gpui::px;
use gpui::*;
use gpui_component::ActiveTheme;
use gpui_component::Disableable;
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::input::{Input, NumberInput};
use gpui_component::{h_flex, v_flex};
use wellbeing_core::{Category, CategoryId, Policy, PolicyKind};

use crate::app::App as GuiApp;
use crate::components::card;
use crate::dbus::DaemonClient;
use crate::theme::{self, rad, sp};

// ---------------------------------------------------------------------------
// Data types (unconditional — no gpui dependency)
// ---------------------------------------------------------------------------

/// Pure-data ViewModel for the Policies screen.
#[derive(Debug, Clone)]
pub struct PoliciesViewModel {
    /// Every app id ever seen in the event log (for the dropdown selector).
    pub app_list: Vec<String>,
    /// Currently-edited policy target + form data, if any.
    pub selected_policy: Option<(PolicyTarget, PolicyConfigForm)>,
    /// All categories from the DB.
    pub categories: Vec<Category>,
    /// All policies for the current user (or all users if admin).
    pub policies: Vec<Policy>,
    /// Per-field validation error messages shown in the PolicyEditor.
    pub validation_errors: Vec<String>,
    /// Whether the current session uid owns admin privileges.
    pub is_admin: bool,
}

/// UI-level target for a policy — mirrors `Policy`'s two variants but carries
/// user-editable form data instead of a finalized domain value.
#[derive(Clone, Debug)]
pub enum PolicyTarget {
    /// Target an individual app by its `AppId` string.
    App(String),
    /// Target every app in a category by the category's row id.
    Category(i64),
}

/// Editable form fields for a single policy configuration.
#[derive(Clone, Debug)]
pub struct PolicyConfigForm {
    /// Policy kind discriminant string: `"Block"`, `"TimeLimit"`, or `"Notify"`.
    pub kind: String,
    /// Per-day time limit in minutes (only meaningful when kind == TimeLimit).
    pub time_limit_minutes: i64,
    /// One-shot extension grant in seconds.
    pub extra_seconds: i64,
    /// JSON-encoded schedule rules (see `TimeWindow` / `ScheduleRule`).
    pub schedule_json: String,
    /// Whether this policy is currently active / enforced.
    pub active: bool,
    /// Target app id (window class for Hyprland). Empty = no app target.
    pub app_id: String,
}

impl Default for PolicyConfigForm {
    fn default() -> Self {
        Self {
            kind: "Block".into(),
            time_limit_minutes: 60,
            extra_seconds: 0,
            schedule_json: "{}".into(),
            active: true,
            app_id: String::new(),
        }
    }
}

/// Build a `PoliciesViewModel` from the raw data sources the D-Bus client /
/// cache provides.
pub fn build_policies_viewmodel(
    policies: &[Policy],
    categories: &[Category],
    app_ids: &[String],
    is_admin: bool,
) -> PoliciesViewModel {
    PoliciesViewModel {
        app_list: app_ids.to_vec(),
        selected_policy: None,
        categories: categories.to_vec(),
        policies: policies.to_vec(),
        validation_errors: Vec::new(),
        is_admin,
    }
}

// ---------------------------------------------------------------------------
// Interactive policy editor (an `impl App` method so callbacks can mutate the
// view's editing state and persist via the daemon client).
// ---------------------------------------------------------------------------

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
                let kind_display = match p.kind {
                    PolicyKind::Block => "Block",
                    PolicyKind::TimeLimit => "Time Limit",
                    PolicyKind::Notify => "Notify",
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
                        let kind = p.kind;
                        let app_id = p.app_id.clone();
                        let cat_id = p.category_id;
                        let tls = p.time_limit_seconds;
                        let extra = p.extra_seconds;
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
                                            PolicyKind::Block => "Block".into(),
                                            PolicyKind::TimeLimit => "TimeLimit".into(),
                                            PolicyKind::Notify => "Notify".into(),
                                        },
                                        time_limit_minutes: tls / 60,
                                        extra_seconds: extra,
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
                            "New policy — pick an app".to_string()
                        } else {
                            format!("Editing app: {}", id)
                        }
                    }
                    PolicyTarget::Category(id) => {
                        let cat_name = vm
                            .categories
                            .iter()
                            .find(|c| c.id == CategoryId(*id))
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

        let show_time_limit = form.kind == "TimeLimit";
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
            // Kind selector
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
            // AppId text input (only for App-targeted policies)
            .when(is_app_target, |el| {
                el.child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(
                            div()
                                .text_xs()
                                .text_color(theme::text_primary(&*cx))
                                .child("App ID (window class):"),
                        )
                        .child(
                            div().flex_1().child(
                                Input::new(
                                    self.app_id_input
                                        .as_ref()
                                        .expect("app_id_input not initialized"),
                                )
                                .cleanable(true),
                            ),
                        ),
                )
            })
            // Time limit — NumberInput with direct typing
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
            // Extra seconds — NumberInput with direct typing
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme::text_primary(&*cx))
                            .child("Extra time (sec):"),
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
                    ),
            )
            // Active toggle
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
            // Action buttons
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
                                    // Snapshot InputState values before entity.update borrows app.
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
                                        // Sync InputState values into form before save.
                                        if let Some((_, ref mut form)) = this.policy_edit {
                                            form.time_limit_minutes = tl;
                                            form.extra_seconds = es;
                                            form.app_id = ai;
                                        }
                                        if let Some((target, form)) = this.policy_edit.clone() {
                                            let uid =
                                                this.state.try_lock().map(|s| s.uid).unwrap_or(0);
                                            let input = policy_input_from(target, &form, uid);
                                            let edit_id = this.policy_edit_id;
                                            let state = this.state.clone();
                                            let client = client.clone();
                                            std::mem::drop(cx2.spawn(async move |this2, cx3| {
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
                                                    let _ = state;
                                                    let _ = this2.update(cx3, |this3, cx4| {
                                                        this3.policy_edit = None;
                                                        this3.policy_edit_id = None;
                                                        cx4.notify();
                                                    });
                                                }
                                            }));
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
                                            let state = this.state.clone();
                                            let client = client.clone();
                                            std::mem::drop(cx2.spawn(async move |this2, cx3| {
                                                let _ = client.delete_policy(id).await;
                                                let _ = state;
                                                let _ = this2.update(cx3, |this3, cx4| {
                                                    this3.policy_edit = None;
                                                    this3.policy_edit_id = None;
                                                    cx4.notify();
                                                });
                                            }));
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

/// Build a `PolicyInput` from the editor form + target.
fn policy_input_from(
    target: PolicyTarget,
    form: &PolicyConfigForm,
    owner_id: u32,
) -> wellbeing_core::PolicyInput {
    let kind = match form.kind.as_str() {
        "TimeLimit" => PolicyKind::TimeLimit,
        "Notify" => PolicyKind::Notify,
        _ => PolicyKind::Block,
    };
    // Use the form's app_id (from text input) when targeting an app.
    let (app_id, category_id) = match target {
        PolicyTarget::App(_) => (form.app_id.clone(), 0),
        PolicyTarget::Category(id) => (String::new(), id),
    };
    wellbeing_core::PolicyInput {
        name: format!("policy-{}", app_cat_label(category_id, &app_id)),
        kind,
        app_id: app_id.clone(),
        category_id,
        time_limit_seconds: form.time_limit_minutes * 60,
        extra_seconds: form.extra_seconds,
        notification_repeat_interval_seconds: 0,
        schedule_json: form.schedule_json.clone(),
        active: form.active,
        owner_id,
    }
}

fn app_cat_label(cat_id: i64, app_id: &str) -> String {
    if cat_id > 0 {
        format!("cat-{}", cat_id)
    } else {
        app_id.to_string()
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

// ---------------------------------------------------------------------------
// Stub for non-gpui path
// ---------------------------------------------------------------------------

/// Stub returned when gpui is not enabled.
#[cfg(not(feature = "gui-gpui"))]
pub fn render_policies_view(_vm: &PoliciesViewModel) -> ! {
    panic!("gpui not enabled (feature gui-gpui is off)")
}
