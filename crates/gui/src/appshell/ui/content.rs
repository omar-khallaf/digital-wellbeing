use gpui::prelude::*;
use gpui::*;
use gpui_component::ActiveTheme;
use gpui_component::spinner::Spinner;
use gpui_component::v_flex;
use wellbeing_core::DateRange;

use crate::dashboard;
use crate::reports;

pub fn dashboard_content(
    cx: &gpui::App,
    vm: &Option<dashboard::DashboardViewModel>,
) -> impl IntoElement {
    match vm.as_ref() {
        Some(vm) => dashboard::render_dashboard_view(cx, vm).into_any_element(),
        None => loading_state(cx).into_any_element(),
    }
}

pub fn reports_content(
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

pub fn loading_state(cx: &gpui::App) -> AnyElement {
    v_flex()
        .h_full()
        .items_center()
        .justify_center()
        .gap_2()
        .child(Spinner::new().color(cx.theme().primary))
        .into_any_element()
}
