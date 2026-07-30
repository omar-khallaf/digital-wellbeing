use gpui::prelude::*;
use gpui::*;
use gpui_component::scroll::ScrollableElement;
use wellbeing_core::DateRange;

use crate::appshell::data::App;
use crate::appshell::domain::Tab;
use crate::reports;
use crate::theme::sp;

use super::content::{DashboardContent, PoliciesContent, ReportsContent};

impl App {
    pub(crate) fn content_area(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        active_tab: Tab,
    ) -> AnyElement {
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
                Tab::Dashboard => RenderOnce::render(
                    DashboardContent {
                        vm: self.dashboard_vm.clone(),
                        apps_list: self.dash_apps_list.clone(),
                        titles_list: self.dash_titles_list.clone(),
                    },
                    window,
                    cx,
                )
                .into_any_element(),
                Tab::Policies => {
                    let entity = cx.entity();
                    let vm = self.policies_vm.clone();
                    let pol_list = self.pol_list.clone();
                    let editor_slot = vm.as_ref().map(|vm| {
                        self.render_editor(window, cx, vm, entity.clone())
                            .into_any_element()
                    });
                    RenderOnce::render(
                        PoliciesContent {
                            vm,
                            pol_list,
                            entity,
                            editor_slot,
                        },
                        window,
                        cx,
                    )
                    .into_any_element()
                }
                Tab::Reports => {
                    let date_opts = reports::DateRangeOptions {
                        show_custom,
                        custom_start_input: custom_start.clone(),
                        custom_end_input: custom_end.clone(),
                        on_preset: Box::new(make_on_range(app_entity.clone())),
                        on_toggle_custom: Box::new(toggle_custom.clone()),
                    };
                    RenderOnce::render(
                        ReportsContent {
                            vm: self.reports_vm.clone(),
                            apps_list: self.rep_apps_list.clone(),
                            titles_list: self.rep_titles_list.clone(),
                            date_opts,
                        },
                        window,
                        cx,
                    )
                    .into_any_element()
                }
            })
            .into_any_element()
    }
}
