//! Policy repository — converts DAO types to domain types.
//!
//! Calls store DAO functions and maps results to [`Policy`] domain
//! types via [`From`] conversions. Owns all schedule-loading I/O that was
//! previously in the domain layer.

use std::collections::HashMap;

use wellbeing_core::{
    AppClass, Category, DomainPattern, Effect as CoreEffect, PolicyId, PolicyInput, TargetType,
    TimeWindow, Uid,
};

use crate::policy::data::insert::{NewPolicy, UpdatePolicy};
use crate::policy::data::models::PolicyTableRow;
use crate::policy::domain::{Effect, Policy, PolicyTarget};
use crate::store::DbPool;
use crate::store::daos::apps::AppsDao;
use crate::store::daos::policies::queries;

/// Repository for policy CRUD and resolution.
///
/// Every method acquires a connection from the pool, delegates to the
/// appropriate store DAO, and converts raw rows to domain types.
pub struct PolicyRepo {
    pool: DbPool,
}

impl PolicyRepo {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    /// Convert a [`PolicyTableRow`] to a domain [`Policy`] with schedules loaded.
    /// Listing path — resolves `app_class_str`, populates schedules. Evaluation
    /// path uses [`From<PolicyTableRow>`] which leaves schedules empty (DAO
    /// pre-filtered).
    fn row_to_policy(
        row: PolicyTableRow,
        app_class_str: String,
        schedules: Vec<TimeWindow>,
    ) -> Policy {
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

        let target = match row.target_type {
            0 => PolicyTarget::App(
                AppClass::new(&app_class_str).unwrap_or_else(|_| AppClass::new("unknown").unwrap()),
            ),
            1 => PolicyTarget::Category(category.unwrap_or(Category::Uncategorized)),
            2 => {
                let d = row.domain_pattern.unwrap_or_default();
                PolicyTarget::Domain(
                    DomainPattern::new(&d)
                        .unwrap_or_else(|_| DomainPattern::new("unknown").unwrap()),
                )
            }
            _ => PolicyTarget::Any,
        };

        Policy {
            id: PolicyId(row.id as i64),
            name: row.name,
            effect,
            target,
            priority: row.priority as u64,
            schedule: schedules,
            time_limit_minutes: row.time_limit_minutes.unwrap_or(0) as u64,
            user_id: row.user_id as u32,
            created_by: row.created_by as u32,
        }
    }

    /// List all policies, optionally filtered by user.
    pub async fn list(&self, caller_root: bool, user_id: i32) -> anyhow::Result<Vec<Policy>> {
        let conn = self.pool.client().await?;
        let rows = queries::read_policies(&conn, caller_root, user_id).await?;

        let app_ids: Vec<i32> = rows.iter().filter_map(|r| r.app_id).collect();
        let app_map: HashMap<i32, String> = AppsDao::batch_load_apps(&conn, &app_ids).await?;

        // Batch-load schedules: one query instead of N+1.
        let policy_ids: Vec<i32> = rows.iter().map(|r| r.id).collect();
        let mut schedule_map: HashMap<i32, Vec<TimeWindow>> = HashMap::new();
        if !policy_ids.is_empty() {
            let sched_rows = queries::load_schedules(&conn, &policy_ids).await?;
            for sr in sched_rows {
                schedule_map
                    .entry(sr.policy_id)
                    .or_default()
                    .push(TimeWindow {
                        start_minute: sr.start_minute as u16,
                        end_minute: sr.end_minute as u16,
                        day_mask: sr.day_mask as u8,
                    });
            }
        }

        let mut results = Vec::with_capacity(rows.len());
        for row in rows {
            let schedules = schedule_map.remove(&row.id).unwrap_or_default();
            let app_class_str = row
                .app_id
                .and_then(|id| app_map.get(&id).cloned())
                .unwrap_or_default();
            results.push(Self::row_to_policy(row, app_class_str, schedules));
        }
        Ok(results)
    }

    /// Create a policy within a transaction.
    pub async fn create(&self, input: PolicyInput, caller: u32) -> anyhow::Result<i64> {
        let mut conn = self.pool.client().await?;
        let tx = conn.transaction().await?;

        let app_id: Option<i32> = if input.target_type == TargetType::App {
            Some(AppsDao::ensure_app(&tx, &input.app_class).await?)
        } else {
            None
        };

        let category: Option<i32> = if input.target_type == TargetType::Category {
            queries::resolve_category_name_to_discriminant(&input.category_name)
        } else {
            None
        };

        let domain_pattern: Option<String> =
            (input.target_type == TargetType::Domain).then(|| input.domain_pattern.to_string());

        let time_limit = match input.effect {
            CoreEffect::TimeLimit | CoreEffect::Notify => {
                (input.time_limit_minutes > 0).then_some(input.time_limit_minutes as i32)
            }
            CoreEffect::Allow | CoreEffect::Block => None,
        };

        let new_policy = NewPolicy {
            name: input.name,
            priority: input.priority as i32,
            effect: i32::from(input.effect),
            target_type: i32::from(input.target_type),
            app_id,
            category,
            domain_pattern,
            time_limit_minutes: time_limit,
            user_id: input.user_id.0 as i32,
            created_by: caller as i32,
        };

        let id = queries::create_policy(&tx, new_policy).await?;
        tx.commit().await?;
        Ok(id.0)
    }

    /// Update a policy; returns `true` if a row was updated.
    pub async fn update(&self, id: PolicyId, input: PolicyInput) -> anyhow::Result<bool> {
        let mut conn = self.pool.client().await?;
        let tx = conn.transaction().await?;

        let app_id: Option<i32> = if input.target_type == TargetType::App {
            Some(AppsDao::ensure_app(&tx, &input.app_class).await?)
        } else {
            None
        };

        let category: Option<i32> = if input.target_type == TargetType::Category {
            queries::resolve_category_name_to_discriminant(&input.category_name)
        } else {
            None
        };

        let domain_pattern: Option<Option<String>> = Some(
            (input.target_type == TargetType::Domain).then(|| input.domain_pattern.to_string()),
        );

        let time_limit = Some(match input.effect {
            CoreEffect::TimeLimit | CoreEffect::Notify => {
                (input.time_limit_minutes > 0).then_some(input.time_limit_minutes as i32)
            }
            CoreEffect::Allow | CoreEffect::Block => None,
        });

        let changes = UpdatePolicy {
            name: Some(input.name),
            priority: Some(input.priority as i32),
            effect: Some(i32::from(input.effect)),
            target_type: Some(i32::from(input.target_type)),
            app_id,
            category,
            domain_pattern,
            time_limit_minutes: time_limit,
        };

        let result = queries::update_policy(&tx, id.0 as i32, changes).await?;
        tx.commit().await?;
        Ok(result)
    }

    /// Delete a policy; returns `true` if a row was deleted.
    pub async fn delete(&self, id: i32) -> anyhow::Result<bool> {
        let conn = self.pool.client().await?;
        queries::delete_policy(&conn, id).await
    }

    /// Get the user_id that owns a policy (for auth checks).
    pub async fn get_owner(&self, id: i32) -> anyhow::Result<i32> {
        let conn = self.pool.client().await?;
        queries::get_policy_user(&conn, id).await
    }

    /// Resolve all active policies for an app (evaluation path).
    ///
    /// Schedules are already filtered by the DAO — the resulting [`Policy`]
    /// domain types have an empty `schedule` field (always active from the
    /// subset that passed schedule filtering).
    pub async fn resolve_for_app(
        &self,
        app_id: i32,
        category_discriminants: &[i32],
        uid: Uid,
        day_mask: i32,
        minute: i32,
    ) -> anyhow::Result<Vec<Policy>> {
        let conn = self.pool.client().await?;
        let rows = queries::resolve_policies_for_app_full(
            &conn,
            app_id,
            category_discriminants,
            uid,
            day_mask,
            minute,
        )
        .await?;
        let policies: Vec<Policy> = rows.into_iter().map(Policy::from).collect();
        tracing::debug!(
            app_id,
            ?uid,
            day_mask,
            minute,
            ?category_discriminants,
            count = policies.len(),
            "resolve_for_app: returning policies"
        );
        Ok(policies)
    }
}
