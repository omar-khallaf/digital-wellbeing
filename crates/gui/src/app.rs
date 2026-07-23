//! App shell — Sidebar navigation, Header, and content routing by tab.
//!
//! The shell detects the effective uid at startup and switches between
//! AdminMode (root, can manage all users) and UserMode (self only). All visual
//! styling is sourced from the active `gpui_component` theme via `crate::theme`.

use std::sync::Arc;

use gpui::prelude::*;
use gpui::px;
use gpui::*;
use gpui_component::ActiveTheme;
use gpui_component::h_flex;
use gpui_component::input::{InputEvent, InputState, NumberInputEvent, StepAction};
use gpui_component::scroll::ScrollableElement as _;
use gpui_component::spinner::Spinner;
use gpui_component::theme::Theme;
use gpui_component::v_flex;
use nix::unistd::Uid;
use wellbeing_core::*;

use crate::dashboard;
use crate::dbus;
use crate::policies;
use crate::reports;
use crate::theme::*;

/// Runtime mode determined by getuid().
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderMode {
    Admin,
    User,
}

impl RenderMode {
    pub fn detect() -> Self {
        if Uid::current().is_root() {
            RenderMode::Admin
        } else {
            RenderMode::User
        }
    }

    pub fn is_admin(&self) -> bool {
        matches!(self, RenderMode::Admin)
    }
}

/// Active tab in the app shell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Dashboard,
    Policies,
    Reports,
}

impl Tab {
    /// All available tabs in display order.
    pub fn all() -> &'static [Tab] {
        &[Tab::Dashboard, Tab::Policies, Tab::Reports]
    }

    pub fn label(&self) -> &'static str {
        match self {
            Tab::Dashboard => "Dashboard",
            Tab::Policies => "Policies",
            Tab::Reports => "Reports",
        }
    }

    pub fn icon(&self) -> &'static str {
        match self {
            Tab::Dashboard => "\u{25cf}",
            Tab::Policies => "\u{2699}",
            Tab::Reports => "\u{1f4ca}",
        }
    }
}

/// Bundle of ViewModels sent from the background refresh loop to the GPUI
/// entity on each data change — keeps the foreground render path single-pass.
#[derive(Debug, Clone)]
pub struct AppViewModels {
    pub dashboard: Option<dashboard::DashboardViewModel>,
    pub policies: Option<policies::PoliciesViewModel>,
    pub reports: Option<reports::ReportsViewModel>,
}

/// Shared state accessible by all screen views.
pub struct AppState {
    pub mode: RenderMode,
    pub uid: u32,
    pub client: crate::dbus::DaemonClient,
    pub selected_range: DateRange,
    pub range_cache: Vec<DailySummary>,
    pub policy_cache: Vec<PolicyData>,
    pub category_cache: Vec<Category>,
    pub app_category_cache: Vec<AppCategoryRow>,
    pub block_cards: Vec<dashboard::BlockCardInfo>,
    pub daemon_available: bool,
    pub connection_status: dbus::ConnectionStatus,
}

/// Top-level app view.
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
                &s.category_cache,
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
        ));

        let pol_vm = Some(policies::build_policies_viewmodel(
            &s.policy_cache,
            &s.category_cache,
            &[], // policies use app_ids from policy_cache now
            s.mode.is_admin(),
        ));

        let rep_vm = Some(reports::build_reports_viewmodel(
            s.selected_range,
            &s.range_cache,
            &s.category_cache,
            &s.app_category_cache,
        ));

        (db_vm, pol_vm, rep_vm)
    }

    fn mode_label(&self) -> &'static str {
        self.state
            .try_lock()
            .map(|s| if s.mode.is_admin() { "Admin" } else { "User" })
            .unwrap_or("User")
    }

    fn connection_status_label(&self) -> String {
        self.state
            .try_lock()
            .map(|s| match s.connection_status {
                dbus::ConnectionStatus::Connected(dbus::BusType::System) => {
                    "Connected (System)".into()
                }
                dbus::ConnectionStatus::Connected(dbus::BusType::Session) => {
                    "Connected (Session)".into()
                }
                dbus::ConnectionStatus::Disconnected => "Disconnected".into(),
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

        // Time limit (minutes) — NumberInput
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

        // Extra seconds — NumberInput
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

        // AppId — plain text Input
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

/// Top-level app view.
impl Render for App {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Lazily create policy editor and custom date range input entities.
        self.ensure_policy_editor_inputs(window, cx);
        self.ensure_custom_range_inputs(window, cx);

        let mode = self.mode_label();
        let active = self.active_tab;
        let entity = cx.entity();

        h_flex()
            .size_full()
            .bg(cx.theme().background)
            .child(sidebar(&*cx, active, mode, self, entity.clone()))
            .child(
                v_flex()
                    .flex_1()
                    .h_full()
                    .min_w(px(0.0))
                    .child(header(&*cx, active, mode))
                    .child(self.content_area(cx, active)),
            )
    }
}

// ── Sidebar ────────────────────────────────────────────────────────────────

fn sidebar(
    cx: &gpui::App,
    active: Tab,
    mode: &str,
    app: &mut App,
    entity: Entity<App>,
) -> AnyElement {
    let items = Tab::all();
    let conn_label = app.connection_status_label();
    v_flex()
        .w(px(220.0))
        .h_full()
        .bg(cx.theme().sidebar)
        .border_r_1()
        .border_color(cx.theme().sidebar_border)
        .child(
            // Brand
            h_flex()
                .px(sp::LG)
                .h(px(56.0))
                .items_center()
                .gap_2()
                .child(
                    div()
                        .size(px(22.0))
                        .rounded(rad::sm())
                        .bg(accent(cx))
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_color(cx.theme().accent_foreground)
                        .text_sm()
                        .font_weight(FontWeight::BOLD)
                        .child("DW"),
                )
                .child(
                    div()
                        .text_base()
                        .font_weight(FontWeight::BOLD)
                        .text_color(cx.theme().sidebar_foreground)
                        .child("Wellbeing"),
                ),
        )
        .child(
            // Nav items
            v_flex().gap_1().p(sp::SM).children(
                items
                    .iter()
                    .map(|tab| nav_item(cx, *tab, *tab == active, app, entity.clone())),
            ),
        )
        .child(
            // Footer: connection status + mode badge + theme toggle
            v_flex()
                .mt_auto()
                .p(sp::LG)
                .gap_2()
                .border_t_1()
                .border_color(cx.theme().sidebar_border)
                // ── Connection status ──────────────────────────────────
                .child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(div().size(px(8.0)).rounded(rad::full()).bg({
                            if conn_label.starts_with("Connected") {
                                success(cx)
                            } else {
                                danger(cx)
                            }
                        }))
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().sidebar_foreground)
                                .child(conn_label.clone()),
                        ),
                )
                // ── Mode badge ────────────────────────────────────────
                .child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(
                            div()
                                .size(px(8.0))
                                .rounded(rad::full())
                                .bg(if mode == "Admin" {
                                    danger(cx)
                                } else {
                                    success(cx)
                                }),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().sidebar_foreground)
                                .child(format!("{} Mode", mode)),
                        ),
                )
                // ── Theme toggle ──────────────────────────────────────
                .child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .cursor_pointer()
                        .on_mouse_down(MouseButton::Left, |_, window, app| {
                            let is_dark =
                                { app.global::<gpui_component::theme::Theme>().is_dark() };
                            let new_mode = if is_dark {
                                gpui_component::theme::ThemeMode::Light
                            } else {
                                gpui_component::theme::ThemeMode::Dark
                            };
                            gpui_component::theme::Theme::change(new_mode, Some(window), app);
                        })
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().sidebar_foreground)
                                .child(if Theme::global(cx).is_dark() {
                                    "\u{2600}"
                                } else {
                                    "\u{263E}"
                                }),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().sidebar_foreground)
                                .hover(|el| el.text_color(cx.theme().sidebar_accent_foreground))
                                .child(if Theme::global(cx).is_dark() {
                                    "Light"
                                } else {
                                    "Dark"
                                }),
                        ),
                ),
        )
        .into_any_element()
}

fn nav_item(
    cx: &gpui::App,
    tab: Tab,
    active: bool,
    _app: &mut App,
    entity: Entity<App>,
) -> AnyElement {
    let label = tab.label();
    let icon = tab.icon();

    div()
        .id(format!("nav-{}", tab as u8))
        .px(sp::MD)
        .py(sp::SM)
        .rounded(rad::md())
        .cursor_pointer()
        .when(active, |el| {
            el.bg(cx.theme().sidebar_accent)
                .text_color(cx.theme().sidebar_accent_foreground)
        })
        .when(!active, |el| {
            el.text_color(cx.theme().sidebar_foreground).hover(|s| {
                s.bg(cx.theme().sidebar_accent)
                    .text_color(cx.theme().sidebar_accent_foreground)
            })
        })
        .on_click({
            let entity = entity.clone();
            move |_, _window, cx2| {
                entity.update(cx2, |this, cx| this.switch_tab(tab, cx));
            }
        })
        .child(
            h_flex()
                .gap_2()
                .items_center()
                .child(div().text_base().child(icon.to_string()))
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .child(label.to_string()),
                ),
        )
        .into_any_element()
}

// ── Header ─────────────────────────────────────────────────────────────────

fn header(cx: &gpui::App, active: Tab, mode: &str) -> AnyElement {
    h_flex()
        .h(px(56.0))
        .px(sp::LG)
        .bg(surface(cx))
        .border_b_1()
        .border_color(border(cx))
        .justify_between()
        .items_center()
        .child(
            h_flex()
                .gap_3()
                .items_center()
                .child(
                    div()
                        .text_lg()
                        .font_weight(FontWeight::BOLD)
                        .text_color(text_primary(cx))
                        .child(active.label().to_string()),
                )
                .child(
                    div()
                        .text_xs()
                        .px(sp::XS)
                        .py(px(1.0))
                        .rounded(rad::sm())
                        .bg(accent(cx))
                        .text_color(cx.theme().accent_foreground)
                        .child(format!("v{}", env!("CARGO_PKG_VERSION"))),
                ),
        )
        .child(
            h_flex().gap_2().items_center().child(
                div()
                    .text_xs()
                    .text_color(text_secondary(cx))
                    .child(format!("{} session", mode)),
            ),
        )
        .into_any_element()
}

// ── Content area ─────────────────────────────────────────────────────────────

impl App {
    fn content_area(&mut self, cx: &mut Context<Self>, active_tab: Tab) -> AnyElement {
        let state = self.state.clone();
        let show_custom = self.show_custom_range;
        let custom_start = self.custom_start_input.clone();
        let custom_end = self.custom_end_input.clone();
        let app_entity = cx.entity();

        // Factory that creates a range-change callback bound to an entity.
        let make_on_range = |app_entity: Entity<Self>| {
            let state = state.clone();
            move |new_range: DateRange, gpui_app: &mut gpui::App| {
                let state = state.clone();
                let entity = app_entity.clone();

                // IMMEDIATE: rebuild reports ViewModel from existing cache so the
                // range label updates right away (no waiting for D-Bus round-trip).
                if let Ok(s) = state.try_lock() {
                    let rep_vm = crate::reports::build_reports_viewmodel(
                        new_range,
                        &s.range_cache,
                        &s.category_cache,
                        &s.app_category_cache,
                    );
                    entity.update(gpui_app, |app, cx| {
                        app.reports_vm = Some(rep_vm);
                        app.show_custom_range = false;
                        cx.notify();
                    });
                }

                // ASYNC: Update persistent state + fetch fresh data in background.
                std::mem::drop(gpui::App::spawn(gpui_app, async move |cx| {
                    // 1. Update selected range
                    state.lock().await.selected_range = new_range;

                    // 2. Fetch fresh usage data for the new range
                    let uid;
                    let start;
                    let end;
                    let client;
                    {
                        let s = state.lock().await;
                        uid = s.uid;
                        start = s.selected_range.start_str();
                        end = s.selected_range.end_str();
                        client = s.client.clone();
                    }
                    client.invalidate_range_cache();
                    if client.connection_status().is_connected()
                        && let Ok(entries) = client.get_usage_range(&start, &end, uid).await
                    {
                        state.lock().await.range_cache = entries;
                    }

                    // 3. Rebuild ViewModels from updated cache. Also close custom
                    //    mode so preset buttons highlight on next render. This runs
                    //    in a single entity update — no race with toggle_custom.
                    let (db, pol, rep) = App::refresh_viewmodels(&state).await;
                    entity.update(cx, |app, cx| {
                        app.apply_viewmodels(AppViewModels {
                            dashboard: db,
                            policies: pol,
                            reports: rep,
                        });
                        app.show_custom_range = false;
                        cx.notify();
                    });
                }));
            }
        };

        let toggle_custom = {
            let state = state.clone();
            let entity = app_entity.clone();
            move |app: &mut gpui::App| {
                let was_custom = entity.read(app).show_custom_range;

                // Toggle synchronously.
                entity.update(app, |this, cx| {
                    this.show_custom_range = !this.show_custom_range;
                    cx.notify();
                });

                // Toggling OFF → revert to Today + refresh data.
                if was_custom {
                    let state = state.clone();
                    let entity = entity.clone();
                    std::mem::drop(gpui::App::spawn(app, async move |cx| {
                        state.lock().await.selected_range = DateRange::last_n_days(1);

                        let (uid, start, end, client) = {
                            let s = state.lock().await;
                            (
                                s.uid,
                                s.selected_range.start_str(),
                                s.selected_range.end_str(),
                                s.client.clone(),
                            )
                        };
                        client.invalidate_range_cache();
                        if client.connection_status().is_connected()
                            && let Ok(entries) = client.get_usage_range(&start, &end, uid).await
                        {
                            state.lock().await.range_cache = entries;
                        }

                        let (db, pol, rep) = App::refresh_viewmodels(&state).await;
                        entity.update(cx, |app, cx| {
                            app.apply_viewmodels(AppViewModels {
                                dashboard: db,
                                policies: pol,
                                reports: rep,
                            });
                            cx.notify();
                        });
                    }));
                }
            }
        };

        div()
            .flex_1()
            .min_h_0()
            .overflow_y_scrollbar()
            .p(sp::LG)
            .child(match active_tab {
                Tab::Dashboard => dashboard_content(cx, &self.dashboard_vm).into_any_element(),
                Tab::Policies => {
                    let vm = self.policies_vm.clone();
                    self.policies_content(cx, &vm).into_any_element()
                }
                Tab::Reports => reports_content(
                    cx,
                    &self.reports_vm,
                    show_custom,
                    custom_start.clone(),
                    custom_end.clone(),
                    make_on_range(app_entity.clone()),
                    toggle_custom.clone(),
                )
                .into_any_element(),
            })
            .into_any_element()
    }

    fn policies_content(
        &mut self,
        cx: &mut Context<Self>,
        vm: &Option<policies::PoliciesViewModel>,
    ) -> impl IntoElement {
        match vm.as_ref() {
            Some(vm) => self.render_policies(cx, vm).into_any_element(),
            None => loading_state(cx).into_any_element(),
        }
    }
}

fn dashboard_content(
    cx: &gpui::App,
    vm: &Option<dashboard::DashboardViewModel>,
) -> impl IntoElement {
    match vm.as_ref() {
        Some(vm) => dashboard::render_dashboard_view(cx, vm).into_any_element(),
        None => loading_state(cx).into_any_element(),
    }
}

fn reports_content(
    cx: &gpui::App,
    vm: &Option<reports::ReportsViewModel>,
    show_custom: bool,
    custom_start: Option<Entity<InputState>>,
    custom_end: Option<Entity<InputState>>,
    on_preset: impl Fn(DateRange, &mut gpui::App) + 'static,
    on_toggle_custom: impl Fn(&mut gpui::App) + 'static,
) -> impl IntoElement {
    match vm.as_ref() {
        Some(vm) => reports::render_reports_view(
            cx,
            vm,
            show_custom,
            custom_start,
            custom_end,
            on_preset,
            on_toggle_custom,
        )
        .into_any_element(),
        None => loading_state(cx).into_any_element(),
    }
}

/// Centered spinner shown while content is loading.
fn loading_state(cx: &gpui::App) -> AnyElement {
    v_flex()
        .h_full()
        .items_center()
        .justify_center()
        .gap_2()
        .child(Spinner::new().color(cx.theme().primary))
        .into_any_element()
}
