use gpui::prelude::*;
use gpui::*;
use gpui_component::ActiveTheme;
use gpui_component::list::ListState;
use gpui_component::v_flex;

use crate::app::App;
use crate::components::{
    CategoriesSection, DashAppsDelegate, DashTitlesDelegate, PolListDelegate, PolicyHeader,
    PolicyListCard, RepAppsDelegate, RepTitlesDelegate,
};
use crate::dashboard;
use crate::policies;
use crate::reports;
use crate::theme;
use crate::theme::sp;

pub struct LoadingState;

impl RenderOnce for LoadingState {
    fn render(self, _: &mut Window, cx: &mut gpui::App) -> impl IntoElement {
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
}

pub struct DashboardContent {
    pub vm: Option<dashboard::DashboardViewModel>,
    pub apps_list: Option<Entity<ListState<DashAppsDelegate>>>,
    pub titles_list: Option<Entity<ListState<DashTitlesDelegate>>>,
}

impl RenderOnce for DashboardContent {
    fn render(self, window: &mut Window, cx: &mut gpui::App) -> impl IntoElement {
        match (&self.vm, &self.apps_list, &self.titles_list) {
            (Some(vm), Some(apps), Some(titles)) => {
                dashboard::render_dashboard_view(window, cx, vm, apps, titles).into_any_element()
            }
            _ => RenderOnce::render(LoadingState, window, cx).into_any_element(),
        }
    }
}

pub struct ReportsContent {
    pub vm: Option<reports::ReportsViewModel>,
    pub apps_list: Option<Entity<ListState<RepAppsDelegate>>>,
    pub titles_list: Option<Entity<ListState<RepTitlesDelegate>>>,
    pub date_opts: reports::DateRangeOptions,
}

impl RenderOnce for ReportsContent {
    fn render(self, window: &mut Window, cx: &mut gpui::App) -> impl IntoElement {
        let Self {
            vm,
            apps_list,
            titles_list,
            date_opts,
        } = self;
        match (&vm, &apps_list, &titles_list) {
            (Some(vm), Some(apps), Some(titles)) => {
                reports::render_reports_view(window, cx, vm, apps, titles, date_opts)
                    .into_any_element()
            }
            _ => RenderOnce::render(LoadingState, window, cx).into_any_element(),
        }
    }
}

/// Editor sub-tree pre-computed in parent's `Render::render()` and passed as
/// `Option<AnyElement>` slot to avoid re-entrant `entity.update()`.
pub struct PoliciesContent {
    pub vm: Option<policies::PoliciesViewModel>,
    pub pol_list: Option<Entity<ListState<PolListDelegate>>>,
    pub entity: Entity<App>,
    pub editor_slot: Option<AnyElement>,
}

impl RenderOnce for PoliciesContent {
    fn render(self, window: &mut Window, cx: &mut gpui::App) -> impl IntoElement {
        let Self {
            vm,
            pol_list,
            entity,
            editor_slot,
        } = self;

        match vm.as_ref() {
            Some(vm) => {
                let loaded = pol_list.is_some();
                v_flex()
                    .gap_4()
                    .child(
                        RenderOnce::render(
                            PolicyHeader {
                                count: vm.policies.len(),
                                entity: entity.clone(),
                            },
                            window,
                            cx,
                        )
                        .into_any_element(),
                    )
                    .child(
                        pol_list
                            .as_ref()
                            .map(|list| {
                                RenderOnce::render(
                                    PolicyListCard {
                                        pol_list: list.clone(),
                                    },
                                    window,
                                    cx,
                                )
                                .into_any_element()
                            })
                            .unwrap_or_else(|| {
                                div()
                                    .py(sp::MD)
                                    .text_sm()
                                    .text_color(theme::text_muted(cx))
                                    .child(if loaded { "Loading..." } else { "" })
                                    .into_any_element()
                            }),
                    )
                    // Slot: pre-rendered editor (or empty) — None skips layout.
                    .children(editor_slot)
                    .child(
                        RenderOnce::render(
                            CategoriesSection {
                                categories: vm.categories.clone(),
                            },
                            window,
                            cx,
                        )
                        .into_any_element(),
                    )
                    .into_any_element()
            }
            _ => RenderOnce::render(LoadingState, window, cx).into_any_element(),
        }
    }
}
