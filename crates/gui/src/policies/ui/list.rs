use gpui::prelude::*;
use gpui::*;
use gpui_component::ActiveTheme;
use gpui_component::scroll::ScrollableElement;

use crate::app::App as GuiApp;
use crate::components::card;
use crate::theme::{self, rad, sp};
use gpui_component::{h_flex, v_flex};

use crate::policies::domain::{PoliciesViewModel, PolicyConfigForm, PolicyTarget};

fn group_label(target_type: wellbeing_core::TargetType) -> &'static str {
    match target_type {
        wellbeing_core::TargetType::App => "App Policies",
        wellbeing_core::TargetType::Category => "Category Policies",
        wellbeing_core::TargetType::Domain => "Domain Policies",
        wellbeing_core::TargetType::Any => "All Targets",
    }
}

#[cfg(feature = "gui-gpui")]
impl GuiApp {
    pub fn render_policy_list(
        &self,
        cx: &mut Context<Self>,
        vm: &PoliciesViewModel,
        entity: Entity<Self>,
    ) -> AnyElement {
        if vm.policies.is_empty() {
            return card(
                &*cx,
                Some("Existing Policies"),
                vec![super::empty_hint(&*cx, "No policies configured yet.")],
            );
        }

        let mut children: Vec<AnyElement> = Vec::new();
        let mut current_group: Option<wellbeing_core::TargetType> = None;

        for p in vm.policies.iter() {
            if current_group != Some(p.target_type) {
                if let Some(_prev) = current_group.take() {
                    children.push(div().h_4().into_any_element());
                }
                current_group = Some(p.target_type);
                children.push(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme::text_label(cx))
                        .px(sp::MD)
                        .py(sp::XS)
                        .child(group_label(p.target_type))
                        .into_any_element(),
                );
            }

            let kind_display = match p.effect {
                wellbeing_core::Effect::Allow => "Allow",
                wellbeing_core::Effect::Block => "Block",
                wellbeing_core::Effect::TimeLimit => "Time Limit",
                wellbeing_core::Effect::Notify => "Notify",
            };
            let target = match p.target_type {
                wellbeing_core::TargetType::App => {
                    format!("App: {}", p.app_class)
                }
                wellbeing_core::TargetType::Category => {
                    let name = if p.category_name.is_empty() {
                        "uncategorized".to_string()
                    } else {
                        p.category_name.clone()
                    };
                    format!("Category: {}", name)
                }
                wellbeing_core::TargetType::Domain => {
                    format!("Domain: {}", p.domain_pattern)
                }
                wellbeing_core::TargetType::Any => "All".to_string(),
            };
            let is_selected = self.policy_edit_id == Some(p.id);

            children.push(
                div()
                    .id(format!("policy-row-{}", p.id.0))
                    .gap_3()
                    .px(sp::MD)
                    .py(sp::SM)
                    .rounded(rad::md())
                    .cursor_pointer()
                    .when(is_selected, |el| el.bg(cx.theme().accent))
                    .when(!is_selected, |el| el.hover(|s| s.bg(cx.theme().border)))
                    .on_click({
                        let entity = entity.clone();
                        let pid = p.id;
                        let effect = p.effect;
                        let target_type = p.target_type;
                        let app_class = p.app_class.clone();
                        let cat_name = p.category_name.clone();
                        let domain_pat = p.domain_pattern.clone();
                        let tls = p.time_limit_minutes;
                        let schedule = p.schedule_json.clone();
                        let priority = p.priority;
                        move |_, _window, app| {
                            entity.update(app, |this, cx2| {
                                this.policy_edit_id = Some(pid);
                                let target = match target_type {
                                    wellbeing_core::TargetType::Category => {
                                        PolicyTarget::Category(cat_name.clone())
                                    }
                                    wellbeing_core::TargetType::Domain => {
                                        PolicyTarget::Domain(domain_pat.clone())
                                    }
                                    wellbeing_core::TargetType::Any => PolicyTarget::Any,
                                    _ => PolicyTarget::App(app_class.clone()),
                                };
                                this.policy_edit = Some((
                                    target,
                                    PolicyConfigForm {
                                        kind: match effect {
                                            wellbeing_core::Effect::Allow => "Allow".into(),
                                            wellbeing_core::Effect::Block => "Block".into(),
                                            wellbeing_core::Effect::TimeLimit => "TimeLimit".into(),
                                            wellbeing_core::Effect::Notify => "Notify".into(),
                                        },
                                        time_limit_minutes: tls,
                                        schedule_json: schedule.clone(),
                                        app_class: app_class.clone(),
                                        category_name: cat_name.clone(),
                                        priority,
                                    },
                                ));
                                cx2.notify();
                            });
                        }
                    })
                    .child(
                        v_flex()
                            .gap_1()
                            .flex_1()
                            .child(
                                h_flex()
                                    .gap_2()
                                    .child(
                                        div()
                                            .text_sm()
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(theme::text_primary(cx))
                                            .child(kind_display.to_string()),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(theme::text_primary(cx))
                                            .child(target),
                                    ),
                            )
                            .child(
                                h_flex()
                                    .gap_2()
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(theme::text_muted(cx))
                                            .child(format!("Priority: {}", p.priority)),
                                    )
                                    .when(
                                        p.target_type == wellbeing_core::TargetType::App
                                            && !p.category_name.is_empty(),
                                        |el| {
                                            el.child(
                                                div()
                                                    .text_xs()
                                                    .text_color(theme::text_muted(cx))
                                                    .child(format!(
                                                        "Category: {}",
                                                        p.category_name
                                                    )),
                                            )
                                        },
                                    ),
                            ),
                    )
                    .into_any_element(),
            );
        }

        let body = v_flex().gap_0().children(children).into_any_element();

        let scroll_body = div().h(px(360.0)).overflow_y_scrollbar().child(body);

        card(
            &*cx,
            Some("Existing Policies"),
            vec![scroll_body.into_any_element()],
        )
    }
}
