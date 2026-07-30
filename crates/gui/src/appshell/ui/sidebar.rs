use gpui::prelude::*;
use gpui::px;
use gpui::*;
use gpui_component::ActiveTheme;
use gpui_component::theme::Theme;
use gpui_component::{h_flex, v_flex};

use crate::appshell::data::App;
use crate::appshell::domain::Tab;
use crate::theme::*;

pub struct Sidebar {
    pub active: Tab,
    pub mode: SharedString,
    pub conn_label: SharedString,
    pub entity: Entity<App>,
}

impl RenderOnce for Sidebar {
    fn render(self, window: &mut Window, cx: &mut gpui::App) -> impl IntoElement {
        let items = Tab::all();
        v_flex()
            .w(px(220.0))
            .h_full()
            .bg(cx.theme().sidebar)
            .border_r_1()
            .border_color(cx.theme().sidebar_border)
            .child(
                h_flex()
                    .px(sp::LG)
                    .h(px(56.0))
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .size(px(22.0))
                            .rounded(rad::sm())
                            .bg(accent(cx))
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_color(cx.theme().accent_foreground)
                            .text_sm()
                            .font_weight(FontWeight::BOLD)
                            .child("DW"),
                    )
                    .child(
                        div()
                            .text_base()
                            .font_weight(FontWeight::BOLD)
                            .text_color(cx.theme().sidebar_foreground)
                            .child("Wellbeing"),
                    ),
            )
            .child({
                let nav_items: Vec<AnyElement> = items
                    .iter()
                    .map(|tab| {
                        RenderOnce::render(
                            NavItem {
                                tab: *tab,
                                active: *tab == self.active,
                                entity: self.entity.clone(),
                            },
                            window,
                            cx,
                        )
                        .into_any_element()
                    })
                    .collect();
                v_flex().gap_1().p(sp::SM).children(nav_items)
            })
            .child(
                v_flex()
                    .mt_auto()
                    .p(sp::LG)
                    .gap_2()
                    .border_t_1()
                    .border_color(cx.theme().sidebar_border)
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(div().size(px(8.0)).rounded(rad::full()).bg({
                                if self.conn_label.starts_with("Connected") {
                                    success(cx)
                                } else {
                                    danger(cx)
                                }
                            }))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().sidebar_foreground)
                                    .child(self.conn_label.clone()),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(div().size(px(8.0)).rounded(rad::full()).bg(
                                if self.mode.as_ref() == "Admin" {
                                    danger(cx)
                                } else {
                                    success(cx)
                                },
                            ))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().sidebar_foreground)
                                    .child(format!("{} Mode", self.mode)),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .cursor_pointer()
                            .on_mouse_down(MouseButton::Left, |_, window, app| {
                                let is_dark =
                                    { app.global::<gpui_component::theme::Theme>().is_dark() };
                                let new_mode = if is_dark {
                                    gpui_component::theme::ThemeMode::Light
                                } else {
                                    gpui_component::theme::ThemeMode::Dark
                                };
                                gpui_component::theme::Theme::change(new_mode, Some(window), app);
                            })
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().sidebar_foreground)
                                    .child(if Theme::global(cx).is_dark() {
                                        "\u{2600}"
                                    } else {
                                        "\u{263E}"
                                    }),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().sidebar_foreground)
                                    .hover(|el| el.text_color(cx.theme().sidebar_accent_foreground))
                                    .child(if Theme::global(cx).is_dark() {
                                        "Light"
                                    } else {
                                        "Dark"
                                    }),
                            ),
                    ),
            )
            .into_any_element()
    }
}

pub struct NavItem {
    pub tab: Tab,
    pub active: bool,
    pub entity: Entity<App>,
}

impl RenderOnce for NavItem {
    fn render(self, _: &mut Window, cx: &mut gpui::App) -> impl IntoElement {
        let label = self.tab.label();
        let icon = self.tab.icon();

        div()
            .id(format!("nav-{}", self.tab as u8))
            .px(sp::MD)
            .py(sp::SM)
            .rounded(rad::md())
            .cursor_pointer()
            .when(self.active, |el| {
                el.bg(cx.theme().sidebar_accent)
                    .text_color(cx.theme().sidebar_accent_foreground)
            })
            .when(!self.active, |el| {
                el.text_color(cx.theme().sidebar_foreground).hover(|s| {
                    s.bg(cx.theme().sidebar_accent)
                        .text_color(cx.theme().sidebar_accent_foreground)
                })
            })
            .on_click({
                let tab = self.tab;
                let entity = self.entity.clone();
                move |_, _window, cx2| {
                    entity.update(cx2, |this, cx| this.switch_tab(tab, cx));
                }
            })
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(div().text_base().child(icon))
                    .child(div().text_sm().font_weight(FontWeight::MEDIUM).child(label)),
            )
            .into_any_element()
    }
}
