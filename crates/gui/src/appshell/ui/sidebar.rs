use gpui::prelude::*;
use gpui::px;
use gpui::*;
use gpui_component::ActiveTheme;
use gpui_component::theme::Theme;
use gpui_component::{h_flex, v_flex};

use crate::appshell::data::App;
use crate::appshell::domain::Tab;
use crate::theme::*;

pub fn sidebar(
    cx: &gpui::App,
    active: Tab,
    mode: &str,
    app: &mut App,
    entity: Entity<App>,
) -> AnyElement {
    let items = Tab::all();
    let conn_label = app.connection_status_label();
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
        .child(
            v_flex().gap_1().p(sp::SM).children(
                items
                    .iter()
                    .map(|tab| nav_item(cx, *tab, *tab == active, app, entity.clone())),
            ),
        )
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
                            if conn_label.starts_with("Connected") {
                                success(cx)
                            } else {
                                danger(cx)
                            }
                        }))
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().sidebar_foreground)
                                .child(conn_label.clone()),
                        ),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(
                            div()
                                .size(px(8.0))
                                .rounded(rad::full())
                                .bg(if mode == "Admin" {
                                    danger(cx)
                                } else {
                                    success(cx)
                                }),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().sidebar_foreground)
                                .child(format!("{} Mode", mode)),
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

fn nav_item(
    cx: &gpui::App,
    tab: Tab,
    active: bool,
    _app: &mut App,
    entity: Entity<App>,
) -> AnyElement {
    let label = tab.label();
    let icon = tab.icon();

    div()
        .id(format!("nav-{}", tab as u8))
        .px(sp::MD)
        .py(sp::SM)
        .rounded(rad::md())
        .cursor_pointer()
        .when(active, |el| {
            el.bg(cx.theme().sidebar_accent)
                .text_color(cx.theme().sidebar_accent_foreground)
        })
        .when(!active, |el| {
            el.text_color(cx.theme().sidebar_foreground).hover(|s| {
                s.bg(cx.theme().sidebar_accent)
                    .text_color(cx.theme().sidebar_accent_foreground)
            })
        })
        .on_click({
            let entity = entity.clone();
            move |_, _window, cx2| {
                entity.update(cx2, |this, cx| this.switch_tab(tab, cx));
            }
        })
        .child(
            h_flex()
                .gap_2()
                .items_center()
                .child(div().text_base().child(icon.to_string()))
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .child(label.to_string()),
                ),
        )
        .into_any_element()
}
