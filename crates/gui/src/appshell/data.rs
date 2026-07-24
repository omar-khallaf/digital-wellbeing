//! App entity — the top-level GPUI view that owns tab state, input entities,
//! and background-task handles.
//!
//! Data-flow methods here: construction, ViewModel application, cache refresh,
//! and InputState lifecycle. Rendering lives in `ui.rs`.

use std::sync::Arc;

use gpui::prelude::*;
use gpui::*;
use gpui_component::input::{InputEvent, InputState, NumberInputEvent, StepAction};

use super::domain::{AppState, AppViewModels, Tab};
use crate::dashboard;
use crate::policies;
use crate::reports;

/// Top-level app view — the single GPUI entity for the entire window.
pub struct App {
    pub(crate) active_tab: Tab,
    pub(crate) state: Arc<tokio::sync::Mutex<AppState>>,
    pub(crate) dashboard_vm: Option<dashboard::DashboardViewModel>,
    pub(crate) policies_vm: Option<policies::PoliciesViewModel>,
    pub(crate) reports_vm: Option<reports::ReportsViewModel>,
    /// Live editing state for the policy editor (target + form).
    pub(crate) policy_edit: Option<(policies::PolicyTarget, policies::PolicyConfigForm)>,
    /// Policy id currently being edited (None = creating new).
    pub(crate) policy_edit_id: Option<wellbeing_core::PolicyId>,
    /// Track last synced policy id to avoid resetting inputs on every render.
    pub(crate) last_synced_policy_edit_id: Option<wellbeing_core::PolicyId>,
    /// InputState entities for the policy editor fields.
    pub(crate) time_limit_input: Option<Entity<InputState>>,
    pub(crate) extra_secs_input: Option<Entity<InputState>>,
    pub(crate) app_id_input: Option<Entity<InputState>>,
    /// Custom date range picker state.
    pub(crate) show_custom_range: bool,
    pub(crate) custom_start_input: Option<Entity<InputState>>,
    pub(crate) custom_end_input: Option<Entity<InputState>>,
    /// Held gpui Task that drains ViewModels from the background signal loop.
    /// Kept alive for the entity's lifetime — dropping it would cancel the task.
    pub(crate) viewmodel_task: Option<gpui::Task<()>>,
    /// Held gpui Task for in-flight policy save/delete operations.
    /// Kept alive until the D-Bus call completes — dropping it would cancel.
    pub(crate) policy_task: Option<gpui::Task<()>>,
}

impl App {
    pub fn new(state: Arc<tokio::sync::Mutex<AppState>>) -> Self {
        // Build initial ViewModels from the (already-populated) state cache so
        // the first render shows real data instead of a loading spinner. Falls
        // back to None if the lock is contended (should never happen at init).
        let (dashboard_vm, policies_vm, reports_vm) = if let Ok(s) = state.try_lock() {
            let db_vm = dashboard::build_dashboard_viewmodel(
                s.selected_range,
                &s.range_cache,
                &s.category_cache,
                &s.app_category_cache,
                s.block_cards.clone(),
                Some(dashboard::build_day_timeline(
                    s.day_events_cache.clone(),
                    chrono::Utc::now().date_naive(),
                    &s.app_category_cache
                        .iter()
                        .map(|ac| (ac.app_id.clone(), ac.display_name.clone()))
                        .collect(),
                )),
            );
            let pol_vm = policies::build_policies_viewmodel(
                &s.policy_cache,
                &s.category_cache,
                &[],
                s.mode.is_admin(),
            );
            let rep_vm = reports::build_reports_viewmodel(
                s.selected_range,
                &s.range_cache,
                &s.app_category_cache,
            );
            (Some(db_vm), Some(pol_vm), Some(rep_vm))
        } else {
            (None, None, None)
        };

        Self {
            active_tab: Tab::Dashboard,
            state,
            dashboard_vm,
            policies_vm,
            reports_vm,
            policy_edit: None,
            policy_edit_id: None,
            last_synced_policy_edit_id: None,
            time_limit_input: None,
            extra_secs_input: None,
            app_id_input: None,
            show_custom_range: false,
            custom_start_input: None,
            custom_end_input: None,
            viewmodel_task: None,
            policy_task: None,
        }
    }

    pub fn switch_tab(&mut self, tab: Tab, cx: &mut Context<Self>) {
        self.active_tab = tab;
        cx.notify();
    }

    /// Store a gpui Task handle to keep it alive. Dropping a Task cancels the
    /// underlying future — the handle must outlive the operation.
    pub fn set_viewmodel_task(&mut self, task: gpui::Task<()>) {
        self.viewmodel_task = Some(task);
    }

    /// Store a policy Task handle (save/delete) to keep it alive.
    pub fn set_policy_task(&mut self, task: gpui::Task<()>) {
        self.policy_task = Some(task);
    }

    /// Apply ViewModels received from the background refresh channel.
    pub fn apply_viewmodels(&mut self, vms: AppViewModels) {
        self.dashboard_vm = vms.dashboard;
        self.policies_vm = vms.policies;
        self.reports_vm = vms.reports;
    }

    /// Refresh all ViewModels from current cache state.
    pub async fn refresh_viewmodels(
        state: &Arc<tokio::sync::Mutex<AppState>>,
    ) -> (
        Option<dashboard::DashboardViewModel>,
        Option<policies::PoliciesViewModel>,
        Option<reports::ReportsViewModel>,
    ) {
        let s = state.lock().await;

        let db_vm = Some(dashboard::build_dashboard_viewmodel(
            s.selected_range,
            &s.range_cache,
            &s.category_cache,
            &s.app_category_cache,
            s.block_cards.clone(),
            Some(dashboard::build_day_timeline(
                s.day_events_cache.clone(),
                chrono::Utc::now().date_naive(),
                &s.app_category_cache
                    .iter()
                    .map(|ac| (ac.app_id.clone(), ac.display_name.clone()))
                    .collect(),
            )),
        ));

        let pol_vm = Some(policies::build_policies_viewmodel(
            &s.policy_cache,
            &s.category_cache,
            &[],
            s.mode.is_admin(),
        ));

        let rep_vm = Some(reports::build_reports_viewmodel(
            s.selected_range,
            &s.range_cache,
            &s.app_category_cache,
        ));

        (db_vm, pol_vm, rep_vm)
    }

    pub(crate) fn mode_label(&self) -> &'static str {
        self.state
            .try_lock()
            .map(|s| if s.mode.is_admin() { "Admin" } else { "User" })
            .unwrap_or("User")
    }

    pub(crate) fn connection_status_label(&self) -> String {
        self.state
            .try_lock()
            .map(|s| match s.connection_status {
                crate::dbus::ConnectionStatus::Connected(crate::dbus::BusType::System) => {
                    "Connected (System)".into()
                }
                crate::dbus::ConnectionStatus::Connected(crate::dbus::BusType::Session) => {
                    "Connected (Session)".into()
                }
                crate::dbus::ConnectionStatus::Disconnected => "Disconnected".into(),
            })
            .unwrap_or_else(|_| "Unknown".into())
    }

    /// Create or reset InputState entities for the custom date range inputs.
    /// Should be called from render() where &mut Window is available.
    pub(crate) fn ensure_custom_range_inputs(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.custom_start_input.is_none() {
            let entity = cx.new(|cx| InputState::new(window, cx).placeholder("YYYY-MM-DD"));
            self.custom_start_input = Some(entity);
        }
        if self.custom_end_input.is_none() {
            let entity = cx.new(|cx| InputState::new(window, cx).placeholder("YYYY-MM-DD"));
            self.custom_end_input = Some(entity);
        }
    }

    /// Create or reset InputState entities for the policy editor fields.
    /// Should be called from render() where &mut Window is available.
    pub(crate) fn ensure_policy_editor_inputs(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((_, form)) = &self.policy_edit else {
            self.time_limit_input = None;
            self.extra_secs_input = None;
            self.app_id_input = None;
            return;
        };
        let form = form.clone();

        let needs_sync = self.last_synced_policy_edit_id != self.policy_edit_id;
        if needs_sync {
            self.last_synced_policy_edit_id = self.policy_edit_id;
        }

        if self.time_limit_input.is_none() {
            let entity: Entity<InputState> =
                cx.new(|cx| InputState::new(window, cx).submit_on_enter(true));
            let _ = cx.subscribe_in(
                &entity,
                window,
                |this: &mut App,
                 state: &Entity<InputState>,
                 event: &NumberInputEvent,
                 window: &mut Window,
                 cx: &mut Context<App>| {
                    match event {
                        NumberInputEvent::Step(StepAction::Increment) => {
                            let cur = state.read(cx).value().parse::<i64>().unwrap_or(0);
                            let new_val = cur + 1;
                            state.update(cx, |input, cx| {
                                input.set_value(new_val.to_string(), window, cx);
                            });
                            if let Some((_, ref mut form)) = this.policy_edit {
                                form.time_limit_minutes = new_val;
                            }
                        }
                        NumberInputEvent::Step(StepAction::Decrement) => {
                            let cur = state.read(cx).value().parse::<i64>().unwrap_or(0);
                            let new_val = (cur - 1).max(0);
                            state.update(cx, |input, cx| {
                                input.set_value(new_val.to_string(), window, cx);
                            });
                            if let Some((_, ref mut form)) = this.policy_edit {
                                form.time_limit_minutes = new_val;
                            }
                        }
                    }
                },
            );
            self.time_limit_input = Some(entity);
        }
        if needs_sync && let Some(ref entity) = self.time_limit_input {
            entity.update(cx, |state, cx| {
                let desired = form.time_limit_minutes.to_string();
                if state.value() != desired.as_str() {
                    state.set_value(desired, window, cx);
                }
            });
        }

        if self.extra_secs_input.is_none() {
            let entity: Entity<InputState> = cx.new(|cx| InputState::new(window, cx));
            let _ = cx.subscribe_in(
                &entity,
                window,
                |this: &mut App,
                 state: &Entity<InputState>,
                 event: &NumberInputEvent,
                 window: &mut Window,
                 cx: &mut Context<App>| {
                    match event {
                        NumberInputEvent::Step(StepAction::Increment) => {
                            let cur = state.read(cx).value().parse::<i64>().unwrap_or(0);
                            let new_val = cur + 1;
                            state.update(cx, |input, cx| {
                                input.set_value(new_val.to_string(), window, cx);
                            });
                            if let Some((_, ref mut form)) = this.policy_edit {
                                form.extra_minutes = new_val;
                            }
                        }
                        NumberInputEvent::Step(StepAction::Decrement) => {
                            let cur = state.read(cx).value().parse::<i64>().unwrap_or(0);
                            let new_val = (cur - 1).max(0);
                            state.update(cx, |input, cx| {
                                input.set_value(new_val.to_string(), window, cx);
                            });
                            if let Some((_, ref mut form)) = this.policy_edit {
                                form.extra_minutes = new_val;
                            }
                        }
                    }
                },
            );
            self.extra_secs_input = Some(entity);
        }
        if needs_sync && let Some(ref entity) = self.extra_secs_input {
            entity.update(cx, |state, cx| {
                let desired = form.extra_minutes.to_string();
                if state.value() != desired.as_str() {
                    state.set_value(desired, window, cx);
                }
            });
        }

        if self.app_id_input.is_none() {
            let entity =
                cx.new(|cx| InputState::new(window, cx).placeholder("e.g. firefox, kitty, Code"));
            let _ = cx.subscribe(
                &entity,
                |this: &mut App,
                 state: Entity<InputState>,
                 event: &InputEvent,
                 cx: &mut Context<App>| {
                    if let InputEvent::Change = event {
                        let val = state.read(cx).value().to_string();
                        if let Some((_, ref mut form)) = this.policy_edit {
                            form.app_id = val;
                        }
                    }
                },
            );
            self.app_id_input = Some(entity);
        }
        if needs_sync && let Some(ref entity) = self.app_id_input {
            entity.update(cx, |state, cx| {
                if state.value() != form.app_id.as_str() {
                    state.set_value(form.app_id.clone(), window, cx);
                }
            });
        }
    }
}
