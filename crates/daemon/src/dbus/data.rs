//! Data access helpers for D-Bus interface methods.

use std::collections::HashMap;

use diesel::{ExpressionMethods, QueryDsl, insert_into, update};
use diesel_async::RunQueryDsl;
use wellbeing_core::{
    AppCategoryRow, Category, CategoryId, DailySummary, DailyUsageEntry, PolicyData, PolicyInput,
    TimeWindow,
};

use crate::policy::data::{NewPolicy, UpdatePolicy};
use crate::policy::{DieselPolicyRepo, PolicyRepo};
use crate::store::DbPool;
use crate::store::schema::{app_categories, categories, daily_usage};

/// Fetch all policies (optionally filtered by owner).
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

/// Create a new policy.
pub(crate) async fn create_policy(
    pool: &DbPool,
    input: PolicyInput,
    caller: u32,
    now_str: &str,
) -> anyhow::Result<i64> {
    let mut conn = pool.get().await?;

    let category_id: Option<i32> = if input.category_id > 0 {
        Some(input.category_id as i32)
    } else {
        None
    };
    let app_id: Option<String> = if input.app_id.is_empty() {
        None
    } else {
        Some(input.app_id.clone())
    };
    let time_limit: Option<i32> = if input.time_limit_minutes > 0 {
        Some(input.time_limit_minutes as i32)
    } else {
        None
    };
    let notify_repeat: Option<i32> = if input.notification_repeat_interval_minutes > 0 {
        Some(input.notification_repeat_interval_minutes as i32)
    } else {
        None
    };

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
        extra_minutes: input.extra_minutes as i32,
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

/// Update an existing policy.
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
        extra_minutes: Some(input.extra_minutes as i32),
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

/// Delete a policy.
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

/// Get daily usage entries for a date and user.
pub(crate) async fn get_daily_usage(
    pool: &DbPool,
    date: &str,
    uid: u32,
) -> anyhow::Result<Vec<DailyUsageEntry>> {
    let mut conn = pool.get().await?;

    let rows: Vec<crate::policy::DailyUsageRow> = daily_usage::table
        .filter(daily_usage::date.eq(date))
        .filter(daily_usage::user_id.eq(uid as i32))
        .select((
            daily_usage::date,
            daily_usage::user_id,
            daily_usage::app_id,
            daily_usage::total_minutes,
            daily_usage::extended,
            daily_usage::updated_at,
        ))
        .load(&mut conn)
        .await?;

    Ok(rows
        .into_iter()
        .map(|r| DailyUsageEntry {
            date: r.date,
            user_id: r.user_id as u32,
            app_id: r.app_id,
            total_minutes: r.total_minutes as i64,
            extended: r.extended,
        })
        .collect())
}

/// Get daily usage grouped by date for a date range.
pub(crate) async fn get_usage_range(
    pool: &DbPool,
    start_date: &str,
    end_date: &str,
    uid: u32,
) -> anyhow::Result<Vec<DailySummary>> {
    let mut conn = pool.get().await?;

    let rows: Vec<crate::policy::DailyUsageRow> = daily_usage::table
        .filter(daily_usage::date.ge(start_date))
        .filter(daily_usage::date.le(end_date))
        .filter(daily_usage::user_id.eq(uid as i32))
        .select((
            daily_usage::date,
            daily_usage::user_id,
            daily_usage::app_id,
            daily_usage::total_minutes,
            daily_usage::extended,
            daily_usage::updated_at,
        ))
        .load(&mut conn)
        .await?;

    let mut grouped: HashMap<String, Vec<DailyUsageEntry>> = HashMap::new();
    for r in rows {
        grouped
            .entry(r.date.clone())
            .or_default()
            .push(DailyUsageEntry {
                date: r.date,
                user_id: r.user_id as u32,
                app_id: r.app_id,
                total_minutes: r.total_minutes as i64,
                extended: r.extended,
            });
    }

    let mut summaries: Vec<DailySummary> = grouped
        .into_iter()
        .map(|(date, entries)| DailySummary {
            date,
            user_id: entries.as_slice().first().map(|e| e.user_id).unwrap_or(uid),
            entries,
        })
        .collect();

    summaries.sort_by(|a, b| a.date.cmp(&b.date));
    Ok(summaries)
}

/// List all categories.
pub(crate) async fn list_categories(pool: &DbPool) -> anyhow::Result<Vec<Category>> {
    let mut conn = pool.get().await?;

    let rows: Vec<(i32, String, Option<String>, Option<String>)> = categories::table
        .select((
            categories::id,
            categories::name,
            categories::color,
            categories::icon,
        ))
        .load(&mut conn)
        .await?;

    Ok(rows
        .into_iter()
        .map(|(id, name, color, icon)| Category {
            id: CategoryId(id as i64),
            name,
            color: color.unwrap_or_default(),
            icon: icon.unwrap_or_default(),
        })
        .collect())
}

/// Get app_categories for caller (user-specific overrides).
pub(crate) async fn get_app_categories(
    pool: &DbPool,
    caller: u32,
) -> anyhow::Result<Vec<AppCategoryRow>> {
    let mut conn = pool.get().await?;

    type AppCategoryRowRaw = (
        String,
        i32,
        Option<i32>,
        Option<String>,
        Option<String>,
        bool,
    );
    let rows: Vec<AppCategoryRowRaw> = app_categories::table
        .filter(app_categories::user_id.eq(caller as i32))
        .select((
            app_categories::app_id,
            app_categories::user_id,
            app_categories::category_id,
            app_categories::display_name,
            app_categories::icon_path,
            app_categories::ignore,
        ))
        .load(&mut conn)
        .await?;

    Ok(rows
        .into_iter()
        .map(
            |(app_id, uid, cat_id, display_name, icon_path, ignore)| AppCategoryRow {
                app_id,
                user_id: uid as u32,
                category_id: cat_id.unwrap_or(0) as i64,
                display_name: display_name.unwrap_or_default(),
                icon_path: icon_path.unwrap_or_default(),
                ignore,
            },
        )
        .collect())
}

/// Set or create an app category override.
pub(crate) async fn set_app_category(
    pool: &DbPool,
    app_id: String,
    category_id: CategoryId,
    caller: u32,
    now_str: &str,
) -> anyhow::Result<()> {
    let mut conn = pool.get().await?;
    let uid = caller as i32;
    let cat_id: Option<i32> = if category_id.0 > 0 {
        Some(category_id.0 as i32)
    } else {
        None
    };

    let updated = update(
        app_categories::table
            .filter(app_categories::app_id.eq(&app_id))
            .filter(app_categories::user_id.eq(uid)),
    )
    .set((
        app_categories::category_id.eq(cat_id),
        app_categories::updated_at.eq(now_str),
    ))
    .execute(&mut conn)
    .await?;

    if updated == 0 {
        insert_into(app_categories::table)
            .values((
                app_categories::app_id.eq(&app_id),
                app_categories::user_id.eq(uid),
                app_categories::category_id.eq(cat_id),
                app_categories::display_name.eq(None::<String>),
                app_categories::icon_path.eq(None::<String>),
                app_categories::ignore.eq(false),
                app_categories::updated_at.eq(now_str),
            ))
            .execute(&mut conn)
            .await?;
    }

    Ok(())
}
