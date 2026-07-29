//! Conversions between domain types and D-Bus / data-layer types.
//!
//! All [`From`] implementations here are pure — they do no I/O.

use wellbeing_core::{
    AppClass, Category, DomainPattern, Effect as CoreEffect, PolicyData, TargetType, Uid,
};

use super::types::*;
use crate::policy::data::models::PolicyTableRow;

impl From<PolicyTableRow> for Policy {
    fn from(row: PolicyTableRow) -> Self {
        let effect = match CoreEffect::try_from(row.effect) {
            Ok(CoreEffect::Allow) => Effect::Allow,
            Ok(CoreEffect::Block) => Effect::Block,
            Ok(CoreEffect::TimeLimit) => Effect::TimeLimit {
                limit_minutes: row.time_limit_minutes.unwrap_or(1) as u64,
            },
            Ok(CoreEffect::Notify) => Effect::Notify {
                limit_minutes: row.time_limit_minutes.unwrap_or(1) as u64,
            },
            Err(_) => {
                tracing::warn!(effect = row.effect, "unknown effect, treating as Block");
                Effect::Block
            }
        };

        let category = row.category.and_then(|v| Category::try_from(v).ok());

        let target = match TargetType::try_from(row.target_type).unwrap_or(TargetType::Any) {
            TargetType::App => PolicyTarget::Any,
            TargetType::Category => {
                PolicyTarget::Category(category.unwrap_or(Category::Uncategorized))
            }
            TargetType::Domain => PolicyTarget::Any,
            TargetType::Any => PolicyTarget::Any,
        };

        Policy {
            id: wellbeing_core::PolicyId(row.id as i64),
            name: row.name,
            effect,
            target,
            priority: row.priority as u64,
            schedule: vec![],
            time_limit_minutes: row.time_limit_minutes.unwrap_or(0) as u64,
            user_id: row.user_id as u32,
            created_by: row.created_by as u32,
        }
    }
}

impl From<Policy> for PolicyData {
    fn from(p: Policy) -> Self {
        let (target_type, app_class, category_name, domain_pattern) = match p.target {
            PolicyTarget::App(ref a) => (
                TargetType::App,
                a.clone(),
                String::new(),
                DomainPattern::new("?").unwrap(),
            ),
            PolicyTarget::Category(cat) => (
                TargetType::Category,
                AppClass::new("?").unwrap(),
                cat.to_string(), // Use Display impl (enum → "Productivity", etc.)
                DomainPattern::new("?").unwrap(),
            ),
            PolicyTarget::Domain(ref d) => (
                TargetType::Domain,
                AppClass::new("?").unwrap(),
                String::new(),
                d.clone(),
            ),
            PolicyTarget::Any => (
                TargetType::Any,
                AppClass::new("?").unwrap(),
                String::new(),
                DomainPattern::new("?").unwrap(),
            ),
        };

        let effect: CoreEffect = p.effect.kind_discriminant();
        let time_limit = match p.effect {
            Effect::TimeLimit { limit_minutes } | Effect::Notify { limit_minutes } => {
                limit_minutes as i64
            }
            _ => 0,
        };

        PolicyData {
            id: p.id,
            name: p.name,
            effect,
            target_type,
            app_class,
            category_name,
            domain_pattern,
            priority: p.priority as i64,
            time_limit_minutes: time_limit,
            schedule_json: serde_json::to_string(&p.schedule).unwrap_or_else(|_| "[]".into()),
            user_id: Uid(p.user_id),
            created_by: Uid(p.created_by),
        }
    }
}
