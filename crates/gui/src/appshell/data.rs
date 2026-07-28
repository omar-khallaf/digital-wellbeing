//! App entity — the top-level GPUI view that owns tab state, input entities,
//! and stores the latest ViewModels received from background flows.

use std::sync::Arc;

use gpui::prelude::*;
use gpui::*;

use gpui_component::input::{InputEvent, InputState, NumberInputEvent, StepAction};
use gpui_component::list::ListState;

use wellbeing_core::AppClass;

use super::domain::{AppState, AppViewModels, Tab};
use crate::components::{
    DashAppsDelegate, DashTitlesDelegate, PolListDelegate, RepAppsDelegate, RepTitlesDelegate,
};
use crate::dashboard;
use crate::policies;
use crate::reports;

/// Top-level app view — the single GPUI entity for the entire window.
pub struct App {
    pub(crate) active_tab: Tab,
    pub(crate) state: Arc<AppState>,
    pub(crate) dashboard_vm: Option<dashboard::DashboardViewModel>,
    pub(crate) policies_vm: Option<policies::PoliciesViewModel>,
    pub(crate) reports_vm: Option<reports::ReportsViewModel>,
    /// Live editing state for the policy editor (target + form).
    pub(crate) policy_edit: Option<(policies::PolicyTarget, policies::PolicyConfigForm)>,
    /// Policy id currently being edited (None = creating new).
    pub(crate) policy_edit_id: Option<wellbeing_core::PolicyId>,
    /// Track last synced policy id to avoid resetting inputs on every render.
    pub(crate) last_synced_policy_edit_id: Option<wellbeing_core::PolicyId>,
    pub(crate) time_limit_input: Option<Entity<InputState>>,
    pub(crate) app_class_input: Option<Entity<InputState>>,
    pub(crate) priority_input: Option<Entity<InputState>>,
    pub(crate) schedule_start_hour: Option<Entity<InputState>>,
    pub(crate) schedule_start_minute: Option<Entity<InputState>>,
    pub(crate) schedule_end_hour: Option<Entity<InputState>>,
    pub(crate) schedule_end_minute: Option<Entity<InputState>>,
    pub(crate) show_custom_range: bool,
    pub(crate) custom_start_input: Option<Entity<InputState>>,
    pub(crate) custom_end_input: Option<Entity<InputState>>,
    /// Held gpui Task for in-flight policy save/delete operations.
    /// Kept alive until the D-Bus call completes — dropping it would cancel.
    pub(crate) policy_task: Option<gpui::Task<()>>,
    /// Long-lived ViewModel receiver task that reads from vm_rx and applies
    /// AppViewModels bundles.
    pub(crate) viewmodel_task: Option<gpui::Task<()>>,
    /// Repository for policy CRUD operations (injected from main).
    pub(crate) policies_repo: Option<crate::policies::data::PoliciesRepo>,
    /// Broadcast sender to trigger the reports background flow refresh.
    pub(crate) reports_refresh_tx: Option<tokio::sync::broadcast::Sender<()>>,
    /// Broadcast sender to trigger the policies background flow refresh.
    pub(crate) pol_refresh_tx: Option<tokio::sync::broadcast::Sender<()>>,

    // ── gpui-component `List` state entities ────────────────────────────
    // Lazily initialised during the first `render()` pass, then reused.

    // Dashboard
    pub(crate) dash_apps_list: Option<Entity<ListState<DashAppsDelegate>>>,
    pub(crate) dash_titles_list: Option<Entity<ListState<DashTitlesDelegate>>>,

    // Reports
    pub(crate) rep_apps_list: Option<Entity<ListState<RepAppsDelegate>>>,
    pub(crate) rep_titles_list: Option<Entity<ListState<RepTitlesDelegate>>>,

    // Policies
    pub(crate) pol_list: Option<Entity<ListState<PolListDelegate>>>,
}

impl App {
    pub fn new(state: Arc<AppState>) -> Self {
        Self {
            active_tab: Tab::Dashboard,
            state,
            dashboard_vm: None,
            policies_vm: None,
            reports_vm: None,
            policy_edit: None,
            policy_edit_id: None,
            last_synced_policy_edit_id: None,
            time_limit_input: None,
            app_class_input: None,
            priority_input: None,
            schedule_start_hour: None,
            schedule_start_minute: None,
            schedule_end_hour: None,
            schedule_end_minute: None,
            show_custom_range: false,
            custom_start_input: None,
            custom_end_input: None,
            policy_task: None,
            viewmodel_task: None,
            policies_repo: None,
            reports_refresh_tx: None,
            pol_refresh_tx: None,

            dash_apps_list: None,
            dash_titles_list: None,
            rep_apps_list: None,
            rep_titles_list: None,
            pol_list: None,
        }
    }

    pub fn switch_tab(&mut self, tab: Tab, cx: &mut Context<Self>) {
        self.active_tab = tab;
        cx.notify();
    }

    pub fn set_policy_task(&mut self, task: gpui::Task<()>) {
        self.policy_task = Some(task);
    }

    /// Store the ViewModel receiver task so it stays alive for the entity's
    /// lifetime. Dropping the task would cancel the receiver.
    pub fn set_viewmodel_task(&mut self, task: gpui::Task<()>) {
        self.viewmodel_task = Some(task);
    }

    pub fn set_policies_repo(&mut self, repo: crate::policies::data::PoliciesRepo) {
        self.policies_repo = Some(repo);
    }

    pub fn set_reports_refresh_tx(&mut self, tx: tokio::sync::broadcast::Sender<()>) {
        self.reports_refresh_tx = Some(tx);
    }

    pub fn set_pol_refresh_tx(&mut self, tx: tokio::sync::broadcast::Sender<()>) {
        self.pol_refresh_tx = Some(tx);
    }

    pub fn apply_viewmodels(&mut self, vms: AppViewModels) {
        if let Some(vm) = &vms.dashboard {
            self.dashboard_vm = Some(vm.clone());
        }
        if let Some(vm) = &vms.policies {
            self.policies_vm = Some(vm.clone());
        }
        if let Some(vm) = &vms.reports {
            self.reports_vm = Some(vm.clone());
        }
    }

    pub(crate) fn mode_label(&self) -> &'static str {
        match self.state.mode {
            super::domain::RenderMode::Admin => "Admin",
            super::domain::RenderMode::User => "User",
        }
    }

    pub(crate) fn connection_status_label(&self) -> String {
        match self.state.connection_status.try_read().ok() {
            Some(status) => match &*status {
                crate::dbus::ConnectionStatus::Connected(crate::dbus::BusType::System) => {
                    "Connected (System)".into()
                }
                crate::dbus::ConnectionStatus::Connected(crate::dbus::BusType::Session) => {
                    "Connected (Session)".into()
                }
                crate::dbus::ConnectionStatus::Disconnected => "Disconnected".into(),
            },
            None => "Unknown".into(),
        }
    }

    /// Ensure date-range InputState entities exist; call from `render()`.
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

    /// Ensure policy-editor InputState entities exist; call from `render()`.
    pub(crate) fn ensure_policy_editor_inputs(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((_, form)) = &self.policy_edit else {
            self.time_limit_input = None;
            self.app_class_input = None;
            self.priority_input = None;
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

        if self.app_class_input.is_none() {
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
                            form.app_class =
                                AppClass::new(&val).unwrap_or_else(|_| form.app_class.clone());
                        }
                    }
                },
            );
            self.app_class_input = Some(entity);
        }
        if needs_sync && let Some(ref entity) = self.app_class_input {
            entity.update(cx, |state, cx| {
                if state.value() != form.app_class.as_str() {
                    state.set_value(form.app_class.to_string(), window, cx);
                }
            });
        }

        if self.priority_input.is_none() {
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
                            let cur = state.read(cx).value().parse::<i64>().unwrap_or(100);
                            let new_val = cur + 1;
                            state.update(cx, |input, cx| {
                                input.set_value(new_val.to_string(), window, cx);
                            });
                            if let Some((_, ref mut form)) = this.policy_edit {
                                form.priority = new_val;
                            }
                        }
                        NumberInputEvent::Step(StepAction::Decrement) => {
                            let cur = state.read(cx).value().parse::<i64>().unwrap_or(100);
                            let new_val = (cur - 1).max(1);
                            state.update(cx, |input, cx| {
                                input.set_value(new_val.to_string(), window, cx);
                            });
                            if let Some((_, ref mut form)) = this.policy_edit {
                                form.priority = new_val;
                            }
                        }
                    }
                },
            );
            self.priority_input = Some(entity);
        }
        if needs_sync && let Some(ref entity) = self.priority_input {
            entity.update(cx, |state, cx| {
                let desired = form.priority.to_string();
                if state.value() != desired.as_str() {
                    state.set_value(desired, window, cx);
                }
            });
        }
    }

    /// Ensure policy-schedule InputState entities exist; call from `render()`.
    pub(crate) fn ensure_policy_schedule_inputs(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.schedule_start_hour.is_none() {
            let entity = cx.new(|cx| InputState::new(window, cx).placeholder("0-23"));
            self.schedule_start_hour = Some(entity);
        }
        if self.schedule_start_minute.is_none() {
            let entity = cx.new(|cx| InputState::new(window, cx).placeholder("0-59"));
            self.schedule_start_minute = Some(entity);
        }
        if self.schedule_end_hour.is_none() {
            let entity = cx.new(|cx| InputState::new(window, cx).placeholder("0-23"));
            self.schedule_end_hour = Some(entity);
        }
        if self.schedule_end_minute.is_none() {
            let entity = cx.new(|cx| InputState::new(window, cx).placeholder("0-59"));
            self.schedule_end_minute = Some(entity);
        }
    }
}
