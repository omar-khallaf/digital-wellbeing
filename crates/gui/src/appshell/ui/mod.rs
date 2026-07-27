//! App shell rendering — sidebar navigation, header, content routing, and
//! empty/loading states.
//!
//! All visual styling is sourced from the active `gpui_component` theme via
//! `crate::theme`.

mod content;
mod header;
mod sidebar;

use self::content::{dashboard_content, loading_state, reports_content};
use self::header::header;
use self::sidebar::sidebar;

use gpui::prelude::*;
use gpui::px;
use gpui::*;
use gpui_component::ActiveTheme;
use gpui_component::scroll::ScrollableElement;
use gpui_component::{h_flex, v_flex};
use wellbeing_core::DateRange;

use crate::policies;
use crate::theme::*;

use super::data::App;
use super::domain::Tab;

impl Render for App {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.ensure_policy_editor_inputs(window, cx);
        self.ensure_policy_schedule_inputs(window, cx);
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

impl App {
    fn content_area(&mut self, cx: &mut Context<Self>, active_tab: Tab) -> AnyElement {
        let state = self.state.clone();
        let state2 = self.state.clone();
        let show_custom = self.show_custom_range;
        let custom_start = self.custom_start_input.clone();
        let custom_end = self.custom_end_input.clone();
        let app_entity = cx.entity();

        let make_on_range = move |app_entity: Entity<Self>| {
            let entity = app_entity.clone();
            move |new_range: DateRange, gpui_app: &mut gpui::App| {
                // Write new range to shared state so background flows pick it up.
                if let Ok(mut guard) = state.selected_range.try_write() {
                    *guard = new_range;
                }
                // Trigger reports flow to re-fetch with the new range.
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
