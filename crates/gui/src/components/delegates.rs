//! gpui-component `ListDelegate` implementations for Dashboard and Reports.
//! Each delegate backs a virtualised `List` in the corresponding screen.

use gpui::prelude::*;
use gpui_component::IndexPath;
use std::sync::Arc;

use super::{
    AppEntryRow, AppEntryView, ListDelegate, ListItem, ListState, TitleEntryRow, TitleEntryView,
};

pub struct DashAppsDelegate {
    pub items: Arc<Vec<AppEntryView>>,
}

impl DashAppsDelegate {
    pub fn new(items: Arc<Vec<AppEntryView>>) -> Self {
        Self { items }
    }
}

impl ListDelegate for DashAppsDelegate {
    type Item = ListItem;

    fn items_count(&self, _section: usize, _cx: &gpui::App) -> usize {
        self.items.len()
    }

    fn render_item(
        &mut self,
        ix: IndexPath,
        _window: &mut gpui::Window,
        cx: &mut gpui::Context<ListState<Self>>,
    ) -> Option<Self::Item> {
        let entry = self.items.get(ix.row)?;
        Some(
            ListItem::new(format!("dash-app-{ix}")).child(
                AppEntryRow {
                    entry: entry.clone(),
                }
                .render(_window, cx)
                .into_any_element(),
            ),
        )
    }

    fn set_selected_index(
        &mut self,
        _ix: Option<IndexPath>,
        _: &mut gpui::Window,
        _: &mut gpui::Context<ListState<Self>>,
    ) {
    }
}

pub struct DashTitlesDelegate {
    pub items: Arc<Vec<TitleEntryView>>,
}

impl DashTitlesDelegate {
    pub fn new(items: Arc<Vec<TitleEntryView>>) -> Self {
        Self { items }
    }
}

impl ListDelegate for DashTitlesDelegate {
    type Item = ListItem;

    fn items_count(&self, _section: usize, _cx: &gpui::App) -> usize {
        self.items.len()
    }

    fn render_item(
        &mut self,
        ix: IndexPath,
        _window: &mut gpui::Window,
        cx: &mut gpui::Context<ListState<Self>>,
    ) -> Option<Self::Item> {
        let entry = self.items.get(ix.row)?;
        Some(
            ListItem::new(format!("dash-title-{ix}")).child(
                TitleEntryRow {
                    entry: entry.clone(),
                }
                .render(_window, cx)
                .into_any_element(),
            ),
        )
    }

    fn set_selected_index(
        &mut self,
        _ix: Option<IndexPath>,
        _: &mut gpui::Window,
        _: &mut gpui::Context<ListState<Self>>,
    ) {
    }
}

pub struct RepAppsDelegate {
    pub items: Arc<Vec<AppEntryView>>,
}

impl RepAppsDelegate {
    pub fn new(items: Arc<Vec<AppEntryView>>) -> Self {
        Self { items }
    }
}

impl ListDelegate for RepAppsDelegate {
    type Item = ListItem;

    fn items_count(&self, _section: usize, _cx: &gpui::App) -> usize {
        self.items.len()
    }

    fn render_item(
        &mut self,
        ix: IndexPath,
        _window: &mut gpui::Window,
        cx: &mut gpui::Context<ListState<Self>>,
    ) -> Option<Self::Item> {
        let entry = self.items.get(ix.row)?;
        Some(
            ListItem::new(format!("rep-app-{ix}")).child(
                AppEntryRow {
                    entry: entry.clone(),
                }
                .render(_window, cx)
                .into_any_element(),
            ),
        )
    }

    fn set_selected_index(
        &mut self,
        _ix: Option<IndexPath>,
        _: &mut gpui::Window,
        _: &mut gpui::Context<ListState<Self>>,
    ) {
    }
}

pub struct RepTitlesDelegate {
    pub items: Arc<Vec<TitleEntryView>>,
}

impl RepTitlesDelegate {
    pub fn new(items: Arc<Vec<TitleEntryView>>) -> Self {
        Self { items }
    }
}

impl ListDelegate for RepTitlesDelegate {
    type Item = ListItem;

    fn items_count(&self, _section: usize, _cx: &gpui::App) -> usize {
        self.items.len()
    }

    fn render_item(
        &mut self,
        ix: IndexPath,
        _window: &mut gpui::Window,
        cx: &mut gpui::Context<ListState<Self>>,
    ) -> Option<Self::Item> {
        let entry = self.items.get(ix.row)?;
        Some(
            ListItem::new(format!("rep-title-{ix}")).child(
                TitleEntryRow {
                    entry: entry.clone(),
                }
                .render(_window, cx)
                .into_any_element(),
            ),
        )
    }

    fn set_selected_index(
        &mut self,
        _ix: Option<IndexPath>,
        _: &mut gpui::Window,
        _: &mut gpui::Context<ListState<Self>>,
    ) {
    }
}
