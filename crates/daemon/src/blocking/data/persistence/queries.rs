//! Lookup queries — category, policy, and usage resolution.

use diesel::{ExpressionMethods, QueryDsl};
use diesel_async::RunQueryDsl;
use wellbeing_core::{AppId, CategoryId, Clock, Uid};

use crate::policy::{DieselPolicyRepo, Policy, PolicyRepo as _};
use crate::store::schema;

use super::events::BlockingRepo;

impl BlockingRepo {
    /// Resolve category ids for an app (user-specific then fallback).
    pub async fn fetch_categories(
        &self,
        app_id: &AppId,
        uid: Uid,
    ) -> anyhow::Result<Vec<CategoryId>> {
        let mut conn = self.pool.get().await?;
        DieselPolicyRepo
            .resolve_categories_for_app(&mut conn, app_id, uid)
            .await
    }

    /// Resolve policies for an app, returning domain [`Policy`] values.
    /// Caller applies schedule filtering via `filter_policies_by_schedule`.
    pub async fn fetch_policies(
        &self,
        app_id: &AppId,
        categories: &[CategoryId],
        uid: Uid,
    ) -> anyhow::Result<Vec<Policy>> {
        let mut conn = self.pool.get().await?;
        let rows = DieselPolicyRepo
            .resolve_policies_for_app(&mut conn, app_id, categories, uid)
            .await?;
        Ok(rows.into_iter().map(|r| r.into_domain_policy()).collect())
    }

    /// Get today's accumulated usage for an app.
    pub async fn fetch_usage(
        &self,
        app_id: &AppId,
        uid: Uid,
        clock: &dyn Clock,
    ) -> anyhow::Result<i64> {
        let mut conn = self.pool.get().await?;
        let today = clock.now().format("%Y-%m-%d").to_string();
        let row: Option<(i32, i32)> = schema::daily_usage::table
            .filter(schema::daily_usage::date.eq(&today))
            .filter(schema::daily_usage::user_id.eq(uid.0 as i32))
            .filter(schema::daily_usage::app_id.eq(app_id.as_ref()))
            .select((
                schema::daily_usage::closed_millis,
                schema::daily_usage::open_millis,
            ))
            .first(&mut conn)
            .await
            .ok();

        let (closed, open) = row.unwrap_or((0, 0));
        Ok(closed as i64 + open as i64)
    }
}
