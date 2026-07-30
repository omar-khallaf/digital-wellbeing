//! Policy-specific components — PolListDelegate with virtualised list sections,
//! plus section components (PolicyHeader, PolicyListCard, CategoriesSection).

mod sections;
pub use sections::*;

use gpui::prelude::*;
use gpui::*;
use gpui_component::IndexPath;
use gpui_component::{h_flex, v_flex};
use std::sync::Arc;

use crate::app::App as GuiApp;
use crate::policies::{PolicyConfigForm, PolicyTarget};
use crate::theme::{self, sp};

use super::{ListDelegate, ListItem, ListState};

/// Delegate for the Policies list with sections per target type.
pub struct PolListDelegate {
    pub app_entity: Entity<GuiApp>,
    pub policies: Arc<Vec<wellbeing_core::PolicyData>>,
    pub selected_id: Option<wellbeing_core::PolicyId>,
}

impl PolListDelegate {
    fn distinct_types(&self) -> Vec<wellbeing_core::TargetType> {
        use wellbeing_core::TargetType;
        let order = [
            TargetType::App,
            TargetType::Category,
            TargetType::Domain,
            TargetType::Any,
        ];
        order
            .into_iter()
            .filter(|t| self.policies.iter().any(|p| p.target_type == *t))
            .collect()
    }
}

impl ListDelegate for PolListDelegate {
    type Item = ListItem;

    fn sections_count(&self, _cx: &gpui::App) -> usize {
        self.distinct_types().len()
    }

    fn items_count(&self, section: usize, _cx: &gpui::App) -> usize {
        let types = self.distinct_types();
        let t = match types.get(section) {
            Some(t) => *t,
            None => return 0,
        };
        self.policies.iter().filter(|p| p.target_type == t).count()
    }

    fn render_section_header(
        &mut self,
        section: usize,
        _: &mut gpui::Window,
        cx: &mut gpui::Context<ListState<Self>>,
    ) -> Option<impl gpui::IntoElement> {
        let types = self.distinct_types();
        let t = *types.get(section)?;
        let label = match t {
            wellbeing_core::TargetType::App => "App Policies",
            wellbeing_core::TargetType::Category => "Category Policies",
            wellbeing_core::TargetType::Domain => "Domain Policies",
            wellbeing_core::TargetType::Any => "All Targets",
        };
        Some(
            gpui::div()
                .text_xs()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(theme::text_label(cx))
                .px(sp::MD)
                .py(sp::XS)
                .child(label),
        )
    }

    fn render_item(
        &mut self,
        ix: IndexPath,
        _: &mut gpui::Window,
        cx: &mut gpui::Context<ListState<Self>>,
    ) -> Option<Self::Item> {
        let types = self.distinct_types();
        let t = *types.get(ix.section)?;
        let global_start = self.policies.iter().position(|p| p.target_type == t)?;
        let p = self.policies.get(global_start + ix.row)?;

        let kind_display = match p.effect {
            wellbeing_core::Effect::Allow => "Allow",
            wellbeing_core::Effect::Block => "Block",
            wellbeing_core::Effect::TimeLimit => "Time Limit",
            wellbeing_core::Effect::Notify => "Notify",
        };
        let target = match p.target_type {
            wellbeing_core::TargetType::App => format!("App: {}", p.app_class),
            wellbeing_core::TargetType::Category => {
                let name = if p.category_name.is_empty() {
                    "uncategorized"
                } else {
                    p.category_name.as_str()
                };
                format!("Category: {}", name)
            }
            wellbeing_core::TargetType::Domain => format!("Domain: {}", p.domain_pattern),
            wellbeing_core::TargetType::Any => "All".to_string(),
        };

        let is_selected = self.selected_id == Some(p.id);
        let app_entity = self.app_entity.clone();
        let pid = p.id;
        let effect = p.effect;
        let target_type = p.target_type;
        let app_class = p.app_class.clone();
        let cat_name = p.category_name.clone();
        let domain_pat = p.domain_pattern.clone();
        let tls = p.time_limit_minutes;
        let schedule = p.schedule_json.clone();
        let priority = p.priority;

        let content = v_flex()
            .gap_1()
            .flex_1()
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        gpui::div()
                            .text_sm()
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .text_color(theme::text_primary(cx))
                            .child(kind_display.to_string()),
                    )
                    .child(
                        gpui::div()
                            .text_xs()
                            .text_color(theme::text_primary(cx))
                            .child(target),
                    ),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        gpui::div()
                            .text_xs()
                            .text_color(theme::text_muted(cx))
                            .child(format!("Priority: {}", priority)),
                    )
                    .when(
                        target_type == wellbeing_core::TargetType::App
                            && !p.category_name.is_empty(),
                        |el| {
                            el.child(
                                gpui::div()
                                    .text_xs()
                                    .text_color(theme::text_muted(cx))
                                    .child(format!("Category: {}", cat_name)),
                            )
                        },
                    ),
            );

        Some(
            ListItem::new(format!("policy-{}", pid.0))
                .selected(is_selected)
                .child(content)
                .on_click(move |_, _window, app| {
                    app_entity.update(app, |this, cx| {
                        if this.policy_edit_id == Some(pid) {
                            this.policy_edit_id = None;
                            this.policy_edit = None;
                            cx.notify();
                            return;
                        }
                        this.policy_edit_id = Some(pid);
                        let tgt = match target_type {
                            wellbeing_core::TargetType::Category => PolicyTarget::Category(
                                wellbeing_core::Category::from_name(&cat_name),
                            ),
                            wellbeing_core::TargetType::Domain => {
                                PolicyTarget::Domain(domain_pat.clone())
                            }
                            wellbeing_core::TargetType::Any => PolicyTarget::Any,
                            _ => PolicyTarget::App(app_class.clone()),
                        };
                        this.policy_edit = Some((
                            tgt,
                            PolicyConfigForm {
                                kind: match effect {
                                    wellbeing_core::Effect::Allow => "Allow".into(),
                                    wellbeing_core::Effect::Block => "Block".into(),
                                    wellbeing_core::Effect::TimeLimit => "TimeLimit".into(),
                                    wellbeing_core::Effect::Notify => "Notify".into(),
                                },
                                time_limit_minutes: tls,
                                schedule_json: schedule.clone(),
                                schedules: serde_json::from_str(&schedule).unwrap_or_default(),
                                app_class: app_class.clone(),
                                category: wellbeing_core::Category::from_name(&cat_name),
                                priority,
                                schedule_new_day_mask: 0x7F,
                            },
                        ));
                        cx.notify();
                    });
                }),
        )
    }

    fn render_empty(
        &mut self,
        _: &mut gpui::Window,
        cx: &mut gpui::Context<ListState<Self>>,
    ) -> impl gpui::IntoElement {
        gpui::div()
            .text_sm()
            .text_color(theme::text_muted(cx))
            .px(sp::MD)
            .py(sp::LG)
            .child("No policies configured yet.")
    }

    fn set_selected_index(
        &mut self,
        _ix: Option<IndexPath>,
        _: &mut gpui::Window,
        _: &mut gpui::Context<ListState<Self>>,
    ) {
    }
}
