use std::sync::Arc;

use gpui::prelude::*;

use crate::components::{AppBadge, AppEntryView, TitleEntryView};
use crate::theme::*;

use crate::appshell::data::App;

impl App {
    /// Pre-compute `AppEntryView` / `TitleEntryView` lists and push them into
    /// the gpui-component `List` delegates so they render fresh data.
    pub fn sync_list_delegates(&mut self, cx: &mut Context<Self>) {
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
}
