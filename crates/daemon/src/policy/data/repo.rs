//! Policy repository — converts DAO types to domain types.
//!
//! Calls store DAO `_conn` functions and maps results to [`Policy`] domain
//! types via [`From`] conversions. Owns all schedule-loading I/O that was
//! previously in the domain layer.

use std::collections::HashMap;

use diesel::ExpressionMethods;
use diesel::QueryDsl;
use diesel_async::AsyncConnection;
use diesel_async::RunQueryDsl;
use wellbeing_core::{
    AppClass, DomainPattern, Effect as CoreEffect, PolicyId, PolicyInput, TargetType, TimeWindow,
    Uid,
};

use crate::policy::data::insert::{NewPolicy, UpdatePolicy};
use crate::policy::data::models::{PolicyRow, ScheduleRow};
use crate::policy::domain::{Effect, Policy, PolicyTarget};
use crate::store::DbPool;
use crate::store::connection::DbConn;
use crate::store::daos::apps::AppsDao;
use crate::store::daos::policies::queries;
use crate::store::schema;

/// Repository for policy CRUD and resolution.
///
/// Every method acquires a connection from the pool, delegates to the
/// appropriate store DAO, and converts raw diesel rows to domain types.
pub struct PolicyRepo {
    pool: DbPool,
}

impl PolicyRepo {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    /// Load schedules from `policy_schedules` for a policy row.
    async fn load_schedules(&self, conn: &mut DbConn, policy_id: i32) -> Vec<TimeWindow> {
        let rows: Vec<ScheduleRow> = schema::policy_schedules::table
            .filter(schema::policy_schedules::policy_id.eq(policy_id))
            .load(conn)
            .await
            .unwrap_or_default();

        rows.into_iter()
            .map(|r| TimeWindow {
                start_minute: r.start_minute as u16,
                end_minute: r.end_minute as u16,
                day_mask: r.day_mask as u8,
            })
            .collect()
    }

    /// Convert a [`PolicyRow`] to a domain [`Policy`] with schedules loaded.
    /// Listing path — resolves `app_class_str`, populates schedules. Evaluation
    /// path uses [`From<PolicyRow>`] which leaves schedules empty (DAO
    /// pre-filtered).
    fn row_to_policy(row: PolicyRow, app_class_str: String, schedules: Vec<TimeWindow>) -> Policy {
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

        let target = match row.target_type {
            0 => PolicyTarget::App(
                AppClass::new(&app_class_str).unwrap_or_else(|_| AppClass::new("unknown").unwrap()),
            ),
            1 => PolicyTarget::Category(row.category_name.unwrap_or_else(|| {
                tracing::warn!(id = row.id, "category_name missing for Category target");
                "Uncategorized".into()
            })),
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
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }

    /// List all policies, optionally filtered by user.
    pub async fn list(&self, caller_root: bool, user_id: i32) -> anyhow::Result<Vec<Policy>> {
        let mut conn = self.pool.get().await?;
        let rows = queries::read_policies(&mut conn, caller_root, user_id).await?;

        let app_ids: Vec<i32> = rows.iter().filter_map(|r| r.app_id).collect();
        let app_map: HashMap<i32, String> = if app_ids.is_empty() {
            HashMap::new()
        } else {
            let app_rows: Vec<(i32, String)> = schema::apps::table
                .filter(schema::apps::id.eq_any(&app_ids))
                .select((schema::apps::id, schema::apps::app_class))
                .load(&mut conn)
                .await?;
            app_rows.into_iter().collect()
        };

        let mut results = Vec::with_capacity(rows.len());
        for row in rows {
            let schedules = self.load_schedules(&mut conn, row.id).await;
            let app_class_str = row
                .app_id
                .and_then(|id| app_map.get(&id).cloned())
                .unwrap_or_default();
            results.push(Self::row_to_policy(row, app_class_str, schedules));
        }
        Ok(results)
    }

    /// Create a policy within a transaction.
    pub async fn create(
        &self,
        input: PolicyInput,
        caller: u32,
        now_str: &str,
    ) -> anyhow::Result<i64> {
        let mut conn = self.pool.get().await?;

        conn.transaction::<_, anyhow::Error, _>(async |conn| {
            let app_id = if input.target_type == TargetType::App {
                Some(AppsDao::ensure_app(conn, &input.app_class).await?)
            } else {
                None
            };

            let category_id = if input.target_type == TargetType::Category {
                queries::resolve_category_name(conn, &input.category_name).await?
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
                category_id,
                domain_pattern,
                time_limit_minutes: time_limit,
                user_id: input.user_id.0 as i32,
                created_by: caller as i32,
                created_at: now_str.to_string(),
                updated_at: now_str.to_string(),
            };

            let id = queries::create_policy(conn, new_policy).await?;
            Ok(id.0)
        })
        .await
    }

    /// Update a policy; returns `true` if a row was updated.
    pub async fn update(
        &self,
        id: PolicyId,
        input: PolicyInput,
        now_str: &str,
    ) -> anyhow::Result<bool> {
        let mut conn = self.pool.get().await?;

        conn.transaction::<_, anyhow::Error, _>(async |conn| {
            let app_id = if input.target_type == TargetType::App {
                Some(Some(AppsDao::ensure_app(conn, &input.app_class).await?))
            } else {
                Some(None)
            };

            let category_id = Some(if input.target_type == TargetType::Category {
                queries::resolve_category_name(conn, &input.category_name).await?
            } else {
                None
            });

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
                category_id,
                domain_pattern,
                time_limit_minutes: time_limit,
                user_id: Some(input.user_id.0 as i32),
                updated_at: now_str.to_string(),
            };

            queries::update_policy(conn, id.0 as i32, changes).await
        })
        .await
    }

    /// Delete a policy; returns `true` if a row was deleted.
    pub async fn delete(&self, id: i32) -> anyhow::Result<bool> {
        let mut conn = self.pool.get().await?;
        queries::delete_policy(&mut conn, id).await
    }

    /// Get the user_id that owns a policy (for auth checks).
    pub async fn get_owner(&self, id: i32) -> anyhow::Result<i32> {
        let mut conn = self.pool.get().await?;
        queries::get_policy_user(&mut conn, id).await
    }

    /// Resolve all active policies for an app (evaluation path).
    ///
    /// Schedules are already filtered by the DAO — the resulting [`Policy`]
    /// domain types have an empty `schedule` field (always active from the
    /// subset that passed schedule filtering).
    pub async fn resolve_for_app(
        &self,
        app_id: i32,
        category_names: &[String],
        uid: Uid,
        day_mask: i32,
        minute: i32,
    ) -> anyhow::Result<Vec<Policy>> {
        let mut conn = self.pool.get().await?;
        let rows = queries::resolve_policies_for_app_full(
            &mut conn,
            app_id,
            category_names,
            uid,
            day_mask,
            minute,
        )
        .await?;
        Ok(rows.into_iter().map(Policy::from).collect())
    }
}
