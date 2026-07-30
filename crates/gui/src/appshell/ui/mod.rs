//! App shell rendering — sidebar navigation, header, content routing, and
//! empty/loading states.
//!
//! All visual styling is sourced from the active `gpui_component` theme via
//! `crate::theme`.

mod content;
mod content_area;
mod header;
mod sidebar;
mod sync;

use std::sync::Arc;

use self::header::Header;
use self::sidebar::Sidebar;

use gpui::prelude::*;
use gpui::px;
use gpui::*;
use gpui_component::ActiveTheme;
use gpui_component::list::ListState;
use gpui_component::{h_flex, v_flex};

use crate::components::{
    DashAppsDelegate, DashTitlesDelegate, PolListDelegate, RepAppsDelegate, RepTitlesDelegate,
};

use super::data::App;

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
            .child(RenderOnce::render(
                Sidebar {
                    active,
                    mode: mode.into(),
                    conn_label: self.connection_status_label().into(),
                    entity: entity.clone(),
                },
                window,
                cx,
            ))
            .child(
                v_flex()
                    .flex_1()
                    .h_full()
                    .min_w(px(0.0))
                    .child(RenderOnce::render(
                        Header {
                            tab: active,
                            mode: mode.into(),
                        },
                        window,
                        cx,
                    ))
                    .child(self.content_area(window, cx, active)),
            )
    }
}
