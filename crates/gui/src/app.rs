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
use gpui_component::input::{InputState, NumberInputEvent, StepAction};
use gpui_component::scroll::ScrollableElement as _;
use gpui_component::theme::Theme;
use gpui_component::v_flex;
use nix::unistd::Uid;
use wellbeing_core::*;

use crate::dashboard;
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
    pub policy_cache: Vec<Policy>,
    pub category_cache: Vec<Category>,
    pub app_category_cache: Vec<AppCategoryRow>,
    pub block_cards: Vec<dashboard::BlockCardInfo>,
    pub daemon_available: bool,
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
    /// InputState entities for the policy editor fields.
    pub(crate) time_limit_input: Option<Entity<InputState>>,
    pub(crate) extra_secs_input: Option<Entity<InputState>>,
    pub(crate) app_id_input: Option<Entity<InputState>>,
}

impl App {
    pub fn new(state: Arc<tokio::sync::Mutex<AppState>>) -> Self {
        let is_admin = state.try_lock().map(|s| s.mode.is_admin()).unwrap_or(false);
        let default_range = DateRange::last_n_days(7);
        Self {
            active_tab: Tab::Dashboard,
            state,
            dashboard_vm: Some(dashboard::DashboardViewModel {
                date_range: default_range,
                bar_chart: Vec::new(),
                pie_app: Vec::new(),
                pie_category: Vec::new(),
                top_apps: Vec::new(),
                block_cards: Vec::new(),
            }),
            policies_vm: Some(policies::PoliciesViewModel {
                app_list: Vec::new(),
                selected_policy: None,
                categories: Vec::new(),
                policies: Vec::new(),
                validation_errors: Vec::new(),
                is_admin,
            }),
            reports_vm: Some(reports::ReportsViewModel {
                date_range: default_range,
                bar_chart: Vec::new(),
                pie_app: Vec::new(),
                total_minutes: 0,
                top_app: String::new(),
            }),
            policy_edit: None,
            policy_edit_id: None,
            time_limit_input: None,
            extra_secs_input: None,
            app_id_input: None,
        }
    }

    pub fn switch_tab(&mut self, tab: Tab, cx: &mut Context<Self>) {
        self.active_tab = tab;
        cx.notify();
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

    /// Create or reset InputState entities for the policy editor fields.
    /// Should be called from render() where &mut Window is available.
    pub(crate) fn ensure_policy_editor_inputs(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.policy_edit.is_none() {
            self.time_limit_input = None;
            self.extra_secs_input = None;
            self.app_id_input = None;
            return;
        }
        let form = self.policy_edit.as_ref().map(|(_, f)| f.clone());
        let Some(form) = form else { return };

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
        if let Some(ref entity) = self.time_limit_input {
            entity.update(cx, |state, cx| {
                state.set_value(form.time_limit_minutes.to_string(), window, cx);
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
                                form.extra_seconds = new_val;
                            }
                        }
                        NumberInputEvent::Step(StepAction::Decrement) => {
                            let cur = state.read(cx).value().parse::<i64>().unwrap_or(0);
                            let new_val = (cur - 1).max(0);
                            state.update(cx, |input, cx| {
                                input.set_value(new_val.to_string(), window, cx);
                            });
                            if let Some((_, ref mut form)) = this.policy_edit {
                                form.extra_seconds = new_val;
                            }
                        }
                    }
                },
            );
            self.extra_secs_input = Some(entity);
        }
        if let Some(ref entity) = self.extra_secs_input {
            entity.update(cx, |state, cx| {
                state.set_value(form.extra_seconds.to_string(), window, cx);
            });
        }

        // AppId — plain text Input
        if self.app_id_input.is_none() {
            let entity =
                cx.new(|cx| InputState::new(window, cx).placeholder("e.g. firefox, kitty, Code"));
            self.app_id_input = Some(entity);
        }
        if let Some(ref entity) = self.app_id_input {
            entity.update(cx, |state, cx| {
                state.set_value(form.app_id.clone(), window, cx);
            });
        }
    }
}

/// Top-level app view.
impl Render for App {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Lazily create policy editor input entities.
        self.ensure_policy_editor_inputs(window, cx);

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
                    .child(disconnected_banner(&*cx, &self.state))
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
            // Footer: mode badge + theme toggle
            v_flex()
                .mt_auto()
                .p(sp::LG)
                .gap_2()
                .border_t_1()
                .border_color(cx.theme().sidebar_border)
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
        let on_range_change = move |new_range: DateRange| {
            if let Ok(mut s) = state.try_lock() {
                s.selected_range = new_range;
            }
        };

        div()
            .flex_1()
            .overflow_y_scrollbar()
            .p(sp::LG)
            .child(match active_tab {
                Tab::Dashboard => {
                    let oc = on_range_change.clone();
                    dashboard_content(cx, &self.dashboard_vm, oc).into_any_element()
                }
                Tab::Policies => {
                    let vm = self.policies_vm.clone();
                    self.policies_content(cx, &vm).into_any_element()
                }
                Tab::Reports => {
                    reports_content(cx, &self.reports_vm, on_range_change).into_any_element()
                }
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
            None => loading_state(cx, "Loading policies…").into_any_element(),
        }
    }
}

fn dashboard_content(
    cx: &gpui::App,
    vm: &Option<dashboard::DashboardViewModel>,
    on_range_change: impl Fn(DateRange) + 'static,
) -> impl IntoElement {
    match vm.as_ref() {
        Some(vm) => dashboard::render_dashboard_view(cx, vm, on_range_change).into_any_element(),
        None => loading_state(cx, "Loading dashboard…").into_any_element(),
    }
}

fn reports_content(
    cx: &gpui::App,
    vm: &Option<reports::ReportsViewModel>,
    on_range_change: impl Fn(DateRange) + 'static,
) -> impl IntoElement {
    match vm.as_ref() {
        Some(vm) => reports::render_reports_view(cx, vm, on_range_change).into_any_element(),
        None => loading_state(cx, "Loading reports…").into_any_element(),
    }
}

/// Warning banner shown when the daemon is unreachable.
fn disconnected_banner(cx: &gpui::App, state: &Arc<tokio::sync::Mutex<AppState>>) -> AnyElement {
    let available = state.try_lock().map(|s| s.daemon_available).unwrap_or(true);

    if available {
        return div().into_any_element();
    }

    div()
        .w_full()
        .px(sp::LG)
        .py(sp::SM)
        .bg(danger(cx))
        .text_color(cx.theme().accent_foreground)
        .child(
            div()
                .text_sm()
                .font_weight(FontWeight::MEDIUM)
                .child("Daemon disconnected — running in degraded mode. Start the daemon to enable full functionality."),
        )
        .into_any_element()
}

/// Centered placeholder shown while a ViewModel is being built.
fn loading_state(cx: &gpui::App, message: &str) -> AnyElement {
    v_flex()
        .h_full()
        .items_center()
        .justify_center()
        .gap_2()
        .child(
            div()
                .text_base()
                .text_color(text_secondary(cx))
                .child(message.to_string()),
        )
        .into_any_element()
}
