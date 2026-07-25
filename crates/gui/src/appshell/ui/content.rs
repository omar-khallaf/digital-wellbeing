use gpui::prelude::*;
use gpui::*;
use gpui_component::ActiveTheme;
use gpui_component::spinner::Spinner;
use gpui_component::v_flex;
use std::sync::Arc;
use wellbeing_core::DateRange;

use crate::appshell::data::App;
use crate::appshell::domain::{AppState, AppViewModels};
use crate::dashboard;
use crate::reports;

/// Shared background refresh: fetch fresh data from D-Bus and rebuild all ViewModels.
/// Used by both range-change and custom-range-toggle callbacks to avoid duplicating
/// the async refresh logic.
pub fn spawn_async_refresh(
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
        client.invalidate_daily_title_cache();
        if client.connection_status().is_connected()
            && let Ok(entries) = client.get_usage_range(&start, &end, uid).await
        {
            state.lock().await.range_cache = entries;
        }
        if client.connection_status().is_connected()
            && let Ok(entries) = client.get_daily_usage_by_title(&start, uid).await
        {
            state.lock().await.title_cache = entries;
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
