//! Conversions between domain types and D-Bus / data-layer types.
//!
//! All [`From`] implementations here are pure — they do no I/O.

use wellbeing_core::{AppClass, DomainPattern, Effect as CoreEffect, PolicyData, TargetType, Uid};

use super::types::*;
use crate::policy::data::models::PolicyRow;

impl From<PolicyRow> for Policy {
    fn from(row: PolicyRow) -> Self {
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

        // NOTE: The SQL WHERE clause in resolve_policies_for_app already
        // filters by target; the PolicyTarget here is used for D-Bus listing
        // and the evaluator's target_matches (which is a secondary check).
        // category_name is populated via LEFT JOIN categories in the eval query.
        // App/Domain map to Any because PolicyRow doesn't carry the resolved
        // app_class_str or domain_pattern needed for a specific PolicyTarget.
        let target = match TargetType::try_from(row.target_type).unwrap_or(TargetType::Any) {
            TargetType::App => PolicyTarget::Any,
            TargetType::Category => {
                PolicyTarget::Category(row.category_name.unwrap_or_else(|| {
                    tracing::warn!(id = row.id, "category_name missing for Category target");
                    "Uncategorized".into()
                }))
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
            created_at: row.created_at,
            updated_at: row.updated_at,
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
            PolicyTarget::Category(ref name) => (
                TargetType::Category,
                AppClass::new("?").unwrap(),
                name.clone(),
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
            created_at: p.created_at,
            updated_at: p.updated_at,
        }
    }
}
