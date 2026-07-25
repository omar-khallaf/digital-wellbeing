//! Policy CRUD helpers for D-Bus interface.

use wellbeing_core::{PolicyData, PolicyInput, TimeWindow};

use crate::policy::data::{NewPolicy, UpdatePolicy};
use crate::policy::{DieselPolicyRepo, PolicyRepo};
use crate::store::DbPool;

pub(crate) async fn list_policies(
    pool: &DbPool,
    caller_root: bool,
    owner_id: i32,
) -> anyhow::Result<Vec<PolicyData>> {
    let mut conn = pool.get().await?;
    let repo = DieselPolicyRepo;
    let rows = repo.read_policies(&mut conn, caller_root, owner_id).await?;
    Ok(rows
        .into_iter()
        .map(|r| PolicyData::from(r.into_domain_policy()))
        .collect())
}

pub(crate) async fn create_policy(
    pool: &DbPool,
    input: PolicyInput,
    caller: u32,
    now_str: &str,
) -> anyhow::Result<i64> {
    let mut conn = pool.get().await?;

    let category_id = (input.category_id > 0).then_some(input.category_id as i32);
    let app_id = (!input.app_id.is_empty()).then(|| input.app_id.clone());
    let time_limit = (input.time_limit_minutes > 0).then_some(input.time_limit_minutes as i32);
    let notify_repeat = (input.notification_repeat_interval_minutes > 0)
        .then_some(input.notification_repeat_interval_minutes as i32);

    let tw: Option<TimeWindow> = if input.schedule_json.is_empty() {
        None
    } else {
        serde_json::from_str(&input.schedule_json).ok().flatten()
    };
    let schedule_start_hour = tw.as_ref().map(|t| t.start_hour as i32);
    let schedule_end_hour = tw.as_ref().map(|t| t.end_hour as i32);
    let schedule_days = tw
        .as_ref()
        .and_then(|t| serde_json::to_string(&t.days).ok())
        .unwrap_or_default();

    let new_policy = NewPolicy {
        name: input.name,
        action: input.action as i32,
        category_id,
        app_id,
        created_by: caller as i32,
        owner_id: input.owner_id as i32,
        time_limit_minutes: time_limit,
        notification_repeat_interval_minutes: notify_repeat,
        schedule_start_hour,
        schedule_end_hour,
        schedule_days,
        active: input.active,
        created_at: now_str.to_string(),
        updated_at: now_str.to_string(),
    };

    let repo = DieselPolicyRepo;
    let id = repo.create_policy(&mut conn, new_policy).await?;
    Ok(id.0 as i64)
}

pub(crate) async fn update_policy(
    pool: &DbPool,
    id: wellbeing_core::PolicyId,
    input: PolicyInput,
    now_str: &str,
) -> anyhow::Result<bool> {
    let mut conn = pool.get().await?;

    let tw: Option<TimeWindow> = if input.schedule_json.is_empty() {
        None
    } else {
        serde_json::from_str(&input.schedule_json).ok().flatten()
    };
    let schedule_start_hour = tw.as_ref().map(|t| Some(t.start_hour as i32));
    let schedule_end_hour = tw.as_ref().map(|t| Some(t.end_hour as i32));
    let schedule_days = Some(
        tw.as_ref()
            .and_then(|t| serde_json::to_string(&t.days).ok())
            .unwrap_or_default(),
    );

    let changes = UpdatePolicy {
        name: Some(input.name),
        action: Some(input.action as i32),
        category_id: Some(if input.category_id > 0 {
            Some(input.category_id as i32)
        } else {
            None
        }),
        app_id: Some(if input.app_id.is_empty() {
            None
        } else {
            Some(input.app_id)
        }),
        time_limit_minutes: Some(if input.time_limit_minutes > 0 {
            Some(input.time_limit_minutes as i32)
        } else {
            None
        }),
        notification_repeat_interval_minutes: Some(
            if input.notification_repeat_interval_minutes > 0 {
                Some(input.notification_repeat_interval_minutes as i32)
            } else {
                None
            },
        ),
        schedule_start_hour,
        schedule_end_hour,
        schedule_days,
        active: Some(input.active),
        updated_at: now_str.to_string(),
    };

    let repo = DieselPolicyRepo;
    repo.update_policy(&mut conn, id.0 as i32, changes).await
}

pub(crate) async fn delete_policy(pool: &DbPool, id: i32) -> anyhow::Result<bool> {
    let mut conn = pool.get().await?;
    let repo = DieselPolicyRepo;
    repo.delete_policy(&mut conn, id).await
}

/// Get policy owner_id (for authorization checks).
pub(crate) async fn get_policy_owner(pool: &DbPool, id: i32) -> anyhow::Result<i32> {
    let mut conn = pool.get().await?;
    let repo = DieselPolicyRepo;
    repo.get_policy_owner(&mut conn, id).await
}
