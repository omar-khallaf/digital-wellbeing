//! Repository trait + impl for policy CRUD and related queries.

use diesel::{
    BoolExpressionMethods, ExpressionMethods, OptionalExtension, QueryDsl, delete, insert_into,
    update,
};
use diesel_async::RunQueryDsl;
use wellbeing_core::{AppId, CategoryId, Clock, PolicyId, Uid};

use crate::store::connection::DbConn;
use crate::store::schema::{app_categories, daily_usage, policies};

use super::insert::{NewPolicy, UpdatePolicy};
use super::models::PolicyRow;

/// Repository trait for policy operations. All methods return concrete
/// types or anyhow::Result — never raw diesel errors at the domain boundary.
#[allow(async_fn_in_trait)]
pub(crate) trait PolicyRepo {
    /// Create a new policy and return its id.
    async fn create_policy(&self, conn: &mut DbConn, new: NewPolicy) -> anyhow::Result<PolicyId>;
    /// Read all policies, optionally filtered by owner.
    async fn read_policies(
        &self,
        conn: &mut DbConn,
        caller_root: bool,
        owner_id: i32,
    ) -> anyhow::Result<Vec<PolicyRow>>;
    /// Update an existing policy.
    async fn update_policy(
        &self,
        conn: &mut DbConn,
        id: i32,
        changes: UpdatePolicy,
    ) -> anyhow::Result<bool>;
    /// Delete a policy by id; returns whether a row was actually deleted.
    async fn delete_policy(&self, conn: &mut DbConn, id: i32) -> anyhow::Result<bool>;
    /// Fetch a single policy row by id.
    async fn get_policy(&self, conn: &mut DbConn, id: i32) -> anyhow::Result<Option<PolicyRow>>;
    /// Resolve policies matching an app (by app_id or category).
    async fn resolve_policies_for_app(
        &self,
        conn: &mut DbConn,
        app_id: &AppId,
        categories: &[CategoryId],
        uid: Uid,
    ) -> anyhow::Result<Vec<PolicyRow>>;
    /// Resolve category ids for an app (user-specific then fallback).
    async fn resolve_categories_for_app(
        &self,
        conn: &mut DbConn,
        app_id: &AppId,
        uid: Uid,
    ) -> anyhow::Result<Vec<CategoryId>>;
    /// Get daily usage for an app on today's date.
    async fn get_daily_usage_for_app(
        &self,
        conn: &mut DbConn,
        app_id: &AppId,
        uid: Uid,
        clock: &dyn Clock,
    ) -> anyhow::Result<Option<(i64, bool)>>;
    /// Get a policy owner_id (for auth checks).
    async fn get_policy_owner(&self, conn: &mut DbConn, id: i32) -> anyhow::Result<i32>;
}

/// Production implementation using diesel-async.
pub(crate) struct DieselPolicyRepo;

impl PolicyRepo for DieselPolicyRepo {
    async fn create_policy(&self, conn: &mut DbConn, new: NewPolicy) -> anyhow::Result<PolicyId> {
        insert_into(policies::table)
            .values(new)
            .execute(conn)
            .await?;

        use diesel::dsl::sql;
        use diesel::sql_types::Integer;
        let last_id: i32 = diesel::select(sql::<Integer>("last_insert_rowid()"))
            .get_result(conn)
            .await?;

        Ok(PolicyId(last_id as i64))
    }

    async fn read_policies(
        &self,
        conn: &mut DbConn,
        caller_root: bool,
        owner_id: i32,
    ) -> anyhow::Result<Vec<PolicyRow>> {
        let rows = if caller_root {
            policies::table.load(conn).await?
        } else {
            policies::table
                .filter(policies::owner_id.eq(owner_id))
                .load(conn)
                .await?
        };
        Ok(rows)
    }

    async fn update_policy(
        &self,
        conn: &mut DbConn,
        id: i32,
        changes: UpdatePolicy,
    ) -> anyhow::Result<bool> {
        let rows = update(policies::table.filter(policies::id.eq(id)))
            .set(changes)
            .execute(conn)
            .await?;
        Ok(rows > 0)
    }

    async fn delete_policy(&self, conn: &mut DbConn, id: i32) -> anyhow::Result<bool> {
        let rows = delete(policies::table.filter(policies::id.eq(id)))
            .execute(conn)
            .await?;
        Ok(rows > 0)
    }

    async fn get_policy(&self, conn: &mut DbConn, id: i32) -> anyhow::Result<Option<PolicyRow>> {
        let row = policies::table
            .filter(policies::id.eq(id))
            .first(conn)
            .await
            .optional()?;
        Ok(row)
    }

    async fn resolve_policies_for_app(
        &self,
        conn: &mut DbConn,
        app_id: &AppId,
        categories: &[CategoryId],
        uid: Uid,
    ) -> anyhow::Result<Vec<PolicyRow>> {
        let cat_ids: Vec<i32> = categories.iter().map(|c| c.0 as i32).collect();

        let rows = if cat_ids.is_empty() {
            policies::table
                .filter(policies::active.eq(true))
                .filter(policies::owner_id.eq(uid.0 as i32))
                .filter(policies::app_id.eq(app_id.as_str()))
                .load(conn)
                .await?
        } else {
            policies::table
                .filter(policies::active.eq(true))
                .filter(policies::owner_id.eq(uid.0 as i32))
                .filter(
                    policies::app_id
                        .eq(app_id.as_str())
                        .or(policies::category_id.eq_any(cat_ids)),
                )
                .load(conn)
                .await?
        };
        Ok(rows)
    }

    async fn resolve_categories_for_app(
        &self,
        conn: &mut DbConn,
        app_id: &AppId,
        uid: Uid,
    ) -> anyhow::Result<Vec<CategoryId>> {
        let rows: Vec<Option<i32>> = app_categories::table
            .filter(app_categories::app_id.eq(app_id.as_str()))
            .filter(app_categories::category_id.is_not_null())
            .filter(app_categories::ignore.eq(false))
            .filter(app_categories::user_id.eq(uid.0 as i32))
            .select(app_categories::category_id)
            .load(conn)
            .await?;

        if !rows.is_empty() {
            return Ok(rows
                .into_iter()
                .flatten()
                .map(|id| CategoryId(id as i64))
                .collect());
        }

        let fallback: Vec<Option<i32>> = app_categories::table
            .filter(app_categories::app_id.eq(app_id.as_str()))
            .filter(app_categories::category_id.is_not_null())
            .filter(app_categories::ignore.eq(false))
            .filter(app_categories::user_id.eq(0i32))
            .select(app_categories::category_id)
            .load(conn)
            .await?;

        Ok(fallback
            .into_iter()
            .flatten()
            .map(|id| CategoryId(id as i64))
            .collect())
    }

    async fn get_daily_usage_for_app(
        &self,
        conn: &mut DbConn,
        app_id: &AppId,
        uid: Uid,
        clock: &dyn Clock,
    ) -> anyhow::Result<Option<(i64, bool)>> {
        let today = clock.now().format("%Y-%m-%d").to_string();

        let result: Option<(i32, bool)> = daily_usage::table
            .filter(daily_usage::user_id.eq(uid.0 as i32))
            .filter(daily_usage::app_id.eq(app_id.as_str()))
            .filter(daily_usage::date.eq(&today))
            .select((daily_usage::total_minutes, daily_usage::extended))
            .first(conn)
            .await
            .optional()?;

        Ok(result.map(|(secs, ext)| (secs as i64, ext)))
    }

    async fn get_policy_owner(&self, conn: &mut DbConn, id: i32) -> anyhow::Result<i32> {
        let owner: i32 = policies::table
            .filter(policies::id.eq(id))
            .select(policies::owner_id)
            .first(conn)
            .await?;
        Ok(owner)
    }
}
