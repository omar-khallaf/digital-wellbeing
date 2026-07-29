use gpui::prelude::*;
use gpui::*;
use gpui_component::ActiveTheme;
use gpui_component::list::ListState;
use gpui_component::v_flex;

use crate::app::App;
use crate::components::{DashAppsDelegate, DashTitlesDelegate, RepAppsDelegate, RepTitlesDelegate};
use crate::dashboard;
use crate::policies;
use crate::reports;

pub fn dashboard_content(
    cx: &gpui::App,
    vm: &Option<dashboard::DashboardViewModel>,
    apps_list: &Option<Entity<ListState<DashAppsDelegate>>>,
    titles_list: &Option<Entity<ListState<DashTitlesDelegate>>>,
) -> impl IntoElement {
    match (vm.as_ref(), apps_list.as_ref(), titles_list.as_ref()) {
        (Some(vm), Some(apps), Some(titles)) => {
            dashboard::render_dashboard_view(cx, vm, apps, titles).into_any_element()
        }
        _ => loading_state(cx).into_any_element(),
    }
}

pub fn reports_content(
    cx: &gpui::App,
    vm: &Option<reports::ReportsViewModel>,
    apps_list: &Option<Entity<ListState<RepAppsDelegate>>>,
    titles_list: &Option<Entity<ListState<RepTitlesDelegate>>>,
    date_opts: reports::DateRangeOptions,
) -> impl IntoElement {
    match (vm.as_ref(), apps_list.as_ref(), titles_list.as_ref()) {
        (Some(vm), Some(apps), Some(titles)) => {
            reports::render_reports_view(cx, vm, apps, titles, date_opts).into_any_element()
        }
        _ => loading_state(cx).into_any_element(),
    }
}

pub fn policies_content(
    this: &mut App,
    cx: &mut Context<App>,
    vm: &Option<policies::PoliciesViewModel>,
) -> impl IntoElement {
    match vm.as_ref() {
        Some(vm) => policies::render_policies_view(this, cx, vm),
        None => v_flex()
            .h_full()
            .items_center()
            .justify_center()
            .gap_2()
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted)
                    .child("Loading..."),
            )
            .into_any_element(),
    }
}

pub fn loading_state(cx: &gpui::App) -> AnyElement {
    v_flex()
        .h_full()
        .items_center()
        .justify_center()
        .gap_2()
        .child(
            div()
                .text_sm()
                .text_color(cx.theme().muted)
                .child("Loading..."),
        )
        .into_any_element()
}
