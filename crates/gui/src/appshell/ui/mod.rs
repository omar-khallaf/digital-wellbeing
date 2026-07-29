//! App shell rendering — sidebar navigation, header, content routing, and
//! empty/loading states.
//!
//! All visual styling is sourced from the active `gpui_component` theme via
//! `crate::theme`.

mod content;
mod header;
mod sidebar;

use std::sync::Arc;

use self::content::{dashboard_content, policies_content, reports_content};
use self::header::header;
use self::sidebar::sidebar;

use gpui::prelude::*;
use gpui::px;
use gpui::*;
use gpui_component::ActiveTheme;
use gpui_component::list::ListState;
use gpui_component::scroll::ScrollableElement;
use gpui_component::{h_flex, v_flex};
use wellbeing_core::DateRange;

use crate::components::{
    AppBadge, AppEntryView, DashAppsDelegate, DashTitlesDelegate, PolListDelegate, RepAppsDelegate,
    RepTitlesDelegate, TitleEntryView,
};
use crate::reports;
use crate::theme::*;

use super::data::App;
use super::domain::Tab;

impl Render for App {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.ensure_policy_editor_inputs(window, cx);
        self.ensure_policy_schedule_inputs(window, cx);
        self.ensure_custom_range_inputs(window, cx);

        // Lazy-init gpui-component List entities (need window from render).
        if self.dash_apps_list.is_none() {
            let delegate = DashAppsDelegate::new(Arc::new(Vec::new()));
            self.dash_apps_list = Some(cx.new(|cx| ListState::new(delegate, window, cx)));
        }
        if self.dash_titles_list.is_none() {
            let delegate = DashTitlesDelegate::new(Arc::new(Vec::new()));
            self.dash_titles_list = Some(cx.new(|cx| ListState::new(delegate, window, cx)));
        }
        if self.rep_apps_list.is_none() {
            let delegate = RepAppsDelegate::new(Arc::new(Vec::new()));
            self.rep_apps_list = Some(cx.new(|cx| ListState::new(delegate, window, cx)));
        }
        if self.rep_titles_list.is_none() {
            let delegate = RepTitlesDelegate::new(Arc::new(Vec::new()));
            self.rep_titles_list = Some(cx.new(|cx| ListState::new(delegate, window, cx)));
        }
        if self.pol_list.is_none() {
            let delegate = PolListDelegate {
                app_entity: cx.entity(),
                policies: Arc::new(Vec::new()),
                selected_id: None,
            };
            self.pol_list = Some(cx.new(|cx| ListState::new(delegate, window, cx)));
        }

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

impl App {
    /// Pre-compute `AppEntryView` / `TitleEntryView` lists and push them into
    /// the gpui-component `List` delegates so they render fresh data.
    fn sync_list_delegates(&mut self, cx: &mut Context<Self>) {
        // ── Dashboard lists ────────────────────────────────────────────
        if let Some(ref vm) = self.dashboard_vm {
            let entries: Arc<Vec<AppEntryView>> = Arc::new(
                vm.top_apps
                    .iter()
                    .map(|e| {
                        let dot_color = e
                            .category_color
                            .as_deref()
                            .and_then(parse_hex)
                            .unwrap_or_else(|| {
                                color_from_str(if e.display_name.is_empty() {
                                    &e.app_class
                                } else {
                                    &e.display_name
                                })
                            });
                        AppEntryView {
                            rank: e.rank,
                            display_name: e.display_name.clone(),
                            total_millis: e.total_millis,
                            percentage: e.percentage,
                            dot_color: Some(dot_color),
                            badge: if e.is_blocked {
                                Some(AppBadge {
                                    text: "BLOCKED".into(),
                                    color: danger(cx),
                                })
                            } else {
                                None
                            },
                        }
                    })
                    .collect(),
            );
            if let Some(list) = &self.dash_apps_list {
                list.update(cx, |state, _| {
                    state.delegate_mut().items = entries;
                });
            }

            let entries: Arc<Vec<TitleEntryView>> = Arc::new(
                vm.top_titles
                    .iter()
                    .map(|e| TitleEntryView {
                        rank: e.rank,
                        app_class: e.app_class.clone(),
                        title: e.title.clone(),
                        total_millis: e.total_millis,
                        percentage: e.percentage,
                    })
                    .collect(),
            );
            if let Some(list) = &self.dash_titles_list {
                list.update(cx, |state, _| {
                    state.delegate_mut().items = entries;
                });
            }
        }

        // ── Reports lists ──────────────────────────────────────────────
        if let Some(ref vm) = self.reports_vm {
            let entries: Arc<Vec<AppEntryView>> = Arc::new(
                vm.app_list
                    .iter()
                    .map(|e| AppEntryView {
                        rank: e.rank,
                        display_name: e.display_name.clone(),
                        total_millis: e.total_millis,
                        percentage: e.percentage,
                        dot_color: None,
                        badge: None,
                    })
                    .collect(),
            );
            if let Some(list) = &self.rep_apps_list {
                list.update(cx, |state, _| {
                    state.delegate_mut().items = entries;
                });
            }

            let entries: Arc<Vec<TitleEntryView>> = Arc::new(
                vm.title_list
                    .iter()
                    .map(|e| TitleEntryView {
                        rank: e.rank,
                        app_class: e.app_class.clone(),
                        title: e.title.clone(),
                        total_millis: e.total_millis,
                        percentage: e.percentage,
                    })
                    .collect(),
            );
            if let Some(list) = &self.rep_titles_list {
                list.update(cx, |state, _| {
                    state.delegate_mut().items = entries;
                });
            }
        }

        // ── Policies list ──────────────────────────────────────────────
        if let Some(ref vm) = self.policies_vm {
            let policies = Arc::new(vm.policies.clone());
            let selected_id = self.policy_edit_id;
            if let Some(list) = &self.pol_list {
                list.update(cx, |state, _| {
                    state.delegate_mut().policies = policies;
                    state.delegate_mut().selected_id = selected_id;
                });
            }
        }
    }

    fn content_area(&mut self, cx: &mut Context<Self>, active_tab: Tab) -> AnyElement {
        // Keep list delegate data in sync with the latest ViewModels.
        self.sync_list_delegates(cx);

        let state = self.state.clone();
        let state2 = self.state.clone();
        let show_custom = self.show_custom_range;
        let custom_start = self.custom_start_input.clone();
        let custom_end = self.custom_end_input.clone();
        let app_entity = cx.entity();

        let make_on_range = move |app_entity: Entity<Self>| {
            let entity = app_entity.clone();
            move |new_range: DateRange, gpui_app: &mut gpui::App| {
                if let Ok(mut guard) = state.selected_range.try_write() {
                    *guard = new_range;
                }
                entity.update(gpui_app, |this, cx| {
                    if let Some(tx) = &this.reports_refresh_tx {
                        let _ = tx.send(());
                    }
                    this.show_custom_range = false;
                    cx.notify();
                });
            }
        };

        let toggle_custom = {
            let entity = app_entity.clone();
            move |app: &mut gpui::App| {
                let was_custom = entity.read(app).show_custom_range;
                entity.update(app, |this, cx| {
                    this.show_custom_range = !this.show_custom_range;
                    cx.notify();
                });
                if was_custom {
                    if let Ok(mut guard) = state2.selected_range.try_write() {
                        *guard = DateRange::last_n_days(7);
                    }
                    entity.update(app, |this, _cx| {
                        if let Some(tx) = &this.reports_refresh_tx {
                            let _ = tx.send(());
                        }
                    });
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
                    dashboard_content(&*cx, &vm, &self.dash_apps_list, &self.dash_titles_list)
                        .into_any_element()
                }
                Tab::Policies => {
                    let vm = self.policies_vm.clone();
                    policies_content(self, cx, &vm).into_any_element()
                }
                Tab::Reports => {
                    let date_opts = reports::DateRangeOptions {
                        show_custom,
                        custom_start_input: custom_start.clone(),
                        custom_end_input: custom_end.clone(),
                        on_preset: Box::new(make_on_range(app_entity.clone())),
                        on_toggle_custom: Box::new(toggle_custom.clone()),
                    };
                    reports_content(
                        &*cx,
                        &self.reports_vm,
                        &self.rep_apps_list,
                        &self.rep_titles_list,
                        date_opts,
                    )
                    .into_any_element()
                }
            })
            .into_any_element()
    }
}
