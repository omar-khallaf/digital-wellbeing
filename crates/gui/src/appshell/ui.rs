//! App shell rendering — sidebar navigation, header, content routing, and
//! empty/loading states.
//!
//! All visual styling is sourced from the active `gpui_component` theme via
//! `crate::theme`.

use gpui::prelude::*;
use gpui::px;
use gpui::*;
use gpui_component::ActiveTheme;
use gpui_component::scroll::ScrollableElement;
use gpui_component::spinner::Spinner;
use gpui_component::theme::Theme;
use gpui_component::{h_flex, v_flex};
use std::sync::Arc;
use wellbeing_core::DateRange;

use crate::dashboard;
use crate::policies;
use crate::reports;
use crate::theme::*;

use super::data::App;
use super::domain::{AppState, AppViewModels, Tab};

// ═════════════════════════════════════════════════════════════════════════════
// Render implementation
// ═════════════════════════════════════════════════════════════════════════════

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

// ═════════════════════════════════════════════════════════════════════════════
// Sidebar
// ═════════════════════════════════════════════════════════════════════════════

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
            v_flex().gap_1().p(sp::SM).children(
                items
                    .iter()
                    .map(|tab| nav_item(cx, *tab, *tab == active, app, entity.clone())),
            ),
        )
        .child(
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

// ═════════════════════════════════════════════════════════════════════════════
// Header
// ═════════════════════════════════════════════════════════════════════════════

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

// ═════════════════════════════════════════════════════════════════════════════
// Content area
// ═════════════════════════════════════════════════════════════════════════════

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
                // Update selected_range synchronously so any concurrent signal
                // handler (e.g. minute-ticker) sees the new range immediately.
                if let Ok(mut s) = state.try_lock() {
                    s.selected_range = new_range;
                    let rep_vm = crate::reports::build_reports_viewmodel(
                        new_range,
                        &s.range_cache,
                        &s.app_category_cache,
                    );
                    entity.update(gpui_app, |app, cx| {
                        app.reports_vm = Some(rep_vm);
                        app.show_custom_range = false;
                        cx.notify();
                    });
                }

                // ASYNC: Update persistent state + fetch fresh data in background.
                spawn_async_refresh(state.clone(), entity.clone(), new_range, gpui_app);
            }
        };

        let toggle_custom = {
            let state = state.clone();
            let entity = app_entity.clone();
            move |app: &mut gpui::App| {
                let was_custom = entity.read(app).show_custom_range;

                entity.update(app, |this, cx| {
                    this.show_custom_range = !this.show_custom_range;
                    cx.notify();
                });

                if was_custom {
                    spawn_async_refresh(
                        state.clone(),
                        entity.clone(),
                        DateRange::last_n_days(1),
                        app,
                    );
                }
            }
        };

        div()
            .flex_1()
            .min_h_0()
            .overflow_y_scrollbar()
            .p(sp::LG)
            .child(match active_tab {
                Tab::Dashboard => {
                    let vm = self.dashboard_vm.clone();
                    dashboard_content(cx, &vm).into_any_element()
                }
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

/// Shared background refresh: fetch fresh data from D-Bus and rebuild all ViewModels.
/// Used by both range-change and custom-range-toggle callbacks to avoid duplicating
/// the async refresh logic.
fn spawn_async_refresh(
    state: Arc<tokio::sync::Mutex<AppState>>,
    entity: Entity<App>,
    new_range: DateRange,
    app: &mut gpui::App,
) {
    std::mem::drop(gpui::App::spawn(app, async move |cx| {
        state.lock().await.selected_range = new_range;

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
    custom_start: Option<Entity<gpui_component::input::InputState>>,
    custom_end: Option<Entity<gpui_component::input::InputState>>,
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
