use gpui::prelude::*;
use gpui::px;
use gpui::*;
use gpui_component::ActiveTheme;
use gpui_component::Disableable;
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::input::{Input, InputState, NumberInput};
use gpui_component::{h_flex, v_flex};

use wellbeing_core::TargetType;

use crate::app::App as GuiApp;
use crate::theme;
use crate::theme::{rad, sp};

use crate::policies::domain::{PoliciesViewModel, PolicyConfigForm, PolicyTarget};
use crate::policies::ui::EmptyHint;

#[cfg(feature = "gui-gpui")]
pub(crate) struct FormBody {
    pub entity: Entity<GuiApp>,
    pub form: PolicyConfigForm,
    pub vm: PoliciesViewModel,
    pub target: PolicyTarget,
    pub is_app_target: bool,
    pub is_cat_target: bool,
    pub show_time_limit: bool,
    pub app_class_input: Entity<InputState>,
    pub time_limit_input: Entity<InputState>,
    pub priority_input: Entity<InputState>,
}

#[cfg(feature = "gui-gpui")]
impl RenderOnce for FormBody {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let entity = self.entity;
        let form = self.form;
        let vm = self.vm;
        let target = self.target;
        let is_app_target = self.is_app_target;
        let is_cat_target = self.is_cat_target;
        let show_time_limit = self.show_time_limit;
        let app_class_input = self.app_class_input;
        let time_limit_input = self.time_limit_input;
        let priority_input = self.priority_input;
        let kinds = ["Allow", "Block", "TimeLimit", "Notify"];
        let kind = &form.kind;

        let target_types: [(TargetType, &str); 3] = [
            (TargetType::App, "App"),
            (TargetType::Category, "Category"),
            (TargetType::Any, "Any"),
        ];
        let current_target_label = match target {
            PolicyTarget::App(_) => TargetType::App,
            PolicyTarget::Category(_) => TargetType::Category,
            PolicyTarget::Domain(_) => TargetType::Domain,
            PolicyTarget::Any => TargetType::Any,
        };

        let body = v_flex()
            .gap_3()
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme::text_primary(cx))
                            .child("Target:"),
                    )
                    .children(target_types.iter().map(|(key, label)| {
                        let is_active = current_target_label == *key;
                        let target_type = *key;
                        let entity = entity.clone();
                        Button::new(format!("target-{:?}", target_type))
                            .label(*label)
                            .when(is_active, |b| b.primary())
                            .on_click(move |_, _window, app| {
                                entity.update(app, |this, cx2| {
                                    if let Some((ref mut t, ref mut f)) = this.policy_edit {
                                        match target_type {
                                            TargetType::Category => {
                                                *t = PolicyTarget::Category(f.category);
                                            }
                                            TargetType::Any => {
                                                *t = PolicyTarget::Any;
                                            }
                                            _ => {
                                                *t = PolicyTarget::App(f.app_class.clone());
                                            }
                                        }
                                    }
                                    cx2.notify();
                                });
                            })
                    })),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme::text_primary(cx))
                            .child("Kind:"),
                    )
                    .children(kinds.iter().map(|k| {
                        let is_active = kind == *k;
                        Button::new(format!("kind-{}", k))
                            .label(*k)
                            .when(is_active, |b| b.primary())
                            .on_click({
                                let entity = entity.clone();
                                let kind_str = (*k).to_string();
                                move |_, _window, app| {
                                    entity.update(app, |this, cx2| {
                                        if let Some((_, f)) = this.policy_edit.as_mut() {
                                            f.kind = kind_str.clone();
                                        }
                                        cx2.notify();
                                    });
                                }
                            })
                    })),
            )
            .when(is_app_target, |el| {
                el.child(
                    h_flex()
                        .w_full()
                        .gap_2()
                        .items_center()
                        .child(
                            div()
                                .text_xs()
                                .text_color(theme::text_primary(cx))
                                .child("App ID (window class):"),
                        )
                        .child(Input::new(&app_class_input).cleanable(true).flex_1()),
                )
            })
            .when(is_cat_target, |el| {
                let cat_rows: Vec<AnyElement> = vm
                    .categories
                    .iter()
                    .map(|cat| {
                        let category = *cat;
                        let is_sel = form.category == category;
                        let color = crate::theme::parse_hex(cat.color())
                            .unwrap_or_else(|| theme::text_muted(cx));
                        let entity = entity.clone();
                        div()
                            .id(format!("cat-opt-{}", category as u8))
                            .gap_2()
                            .px(sp::MD)
                            .py(sp::SM)
                            .rounded(rad::md())
                            .cursor_pointer()
                            .when(is_sel, |el| el.bg(cx.theme().accent))
                            .when(!is_sel, |el| el.hover(|s| s.bg(cx.theme().border)))
                            .on_click(move |_, _window, app| {
                                entity.update(app, |this, cx2| {
                                    if let Some((ref mut t, ref mut f)) = this.policy_edit {
                                        f.category = category;
                                        *t = PolicyTarget::Category(category);
                                    }
                                    cx2.notify();
                                });
                            })
                            .child(
                                h_flex()
                                    .gap_2()
                                    .child(div().size(px(12.0)).rounded(px(2.0)).bg(color))
                                    .child(
                                        div()
                                            .text_sm()
                                            .text_color(theme::text_primary(cx))
                                            .child(cat.name()),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(theme::text_muted(cx))
                                            .child(cat.icon()),
                                    ),
                            )
                            .into_any_element()
                    })
                    .collect();

                let cat_section = if cat_rows.is_empty() {
                    RenderOnce::render(
                        EmptyHint {
                            message: "No categories available.".into(),
                        },
                        window,
                        cx,
                    )
                    .into_any_element()
                } else {
                    v_flex().gap_1().children(cat_rows).into_any_element()
                };

                el.child(
                    v_flex()
                        .gap_1()
                        .child(
                            div()
                                .text_xs()
                                .text_color(theme::text_primary(cx))
                                .child("Category:"),
                        )
                        .child(cat_section),
                )
            })
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme::text_primary(cx))
                            .child("Time limit (min):"),
                    )
                    .child(
                        div().w(px(140.0)).child(
                            NumberInput::new(&time_limit_input)
                                .appearance(true)
                                .disabled(false),
                        ),
                    )
                    .when(!show_time_limit, |el| el.opacity(0.4)),
            )
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme::text_primary(cx))
                            .child("Priority (lower = first):"),
                    )
                    .child(
                        div().w(px(140.0)).child(
                            NumberInput::new(&priority_input)
                                .appearance(true)
                                .disabled(false),
                        ),
                    ),
            );

        body.into_any_element()
    }
}
