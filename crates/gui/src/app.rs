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
use gpui_component::scroll::ScrollableElement as _;
use gpui_component::v_flex;
use nix::unistd::Uid;
use wellbeing_core::*;

use crate::screens::{Tab, dashboard, policies, reports};
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

/// Shared state accessible by all screen views.
pub struct AppState {
    pub mode: RenderMode,
    pub uid: u32,
    pub client: crate::dbus::DaemonClient,
    pub usage_cache: Vec<DailyUsageEntry>,
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
}

impl App {
    pub fn new(state: Arc<tokio::sync::Mutex<AppState>>) -> Self {
        Self {
            active_tab: Tab::Dashboard,
            state,
            dashboard_vm: None,
            policies_vm: None,
            reports_vm: None,
            policy_edit: None,
            policy_edit_id: None,
        }
    }

    pub fn switch_tab(&mut self, tab: Tab, cx: &mut Context<Self>) {
        self.active_tab = tab;
        cx.notify();
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
            &s.usage_cache,
            &s.category_cache,
            &s.app_category_cache,
        ));

        let app_ids: Vec<String> = s
            .usage_cache
            .iter()
            .map(|e| e.app_id.clone())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();

        let pol_vm = Some(policies::build_policies_viewmodel(
            &s.policy_cache,
            &s.category_cache,
            &app_ids,
            s.mode.is_admin(),
        ));

        let rep_vm = Some(reports::build_reports_viewmodel(
            &s.usage_cache,
            &s.category_cache,
            &s.app_category_cache,
            7,
        ));

        (db_vm, pol_vm, rep_vm)
    }

    fn mode_label(&self) -> &'static str {
        self.state
            .try_lock()
            .map(|s| if s.mode.is_admin() { "Admin" } else { "User" })
            .unwrap_or("User")
    }
}

impl Render for App {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
            // Footer: mode badge
            v_flex()
                .mt_auto()
                .p(sp::LG)
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
        div()
            .flex_1()
            .overflow_y_scrollbar()
            .p(sp::LG)
            .child(match active_tab {
                Tab::Dashboard => dashboard_content(cx, &self.dashboard_vm).into_any_element(),
                Tab::Policies => {
                    let vm = self.policies_vm.clone();
                    self.policies_content(cx, &vm).into_any_element()
                }
                Tab::Reports => reports_content(cx, &self.reports_vm).into_any_element(),
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
) -> impl IntoElement {
    match vm.as_ref() {
        Some(vm) => dashboard::render_dashboard_view(cx, vm).into_any_element(),
        None => loading_state(cx, "Loading dashboard…").into_any_element(),
    }
}

fn reports_content(cx: &gpui::App, vm: &Option<reports::ReportsViewModel>) -> impl IntoElement {
    match vm.as_ref() {
        Some(vm) => reports::render_reports_view(cx, vm).into_any_element(),
        None => loading_state(cx, "Loading reports…").into_any_element(),
    }
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
