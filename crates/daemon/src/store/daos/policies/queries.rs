//! Internal query helpers for the policy repository (take an explicit
//! `&DbConn` argument so the transaction methods can share a connection).

use std::collections::HashMap;

use turso::params_from_iter;
use wellbeing_core::Uid;

use crate::policy::data::insert::{NewPolicy, UpdatePolicy};
use crate::policy::data::models::{PolicyTableRow, ScheduleRow};
use crate::store::connection::DbConn;
use crate::store::schema_constants::{policies, policy_schedules};

/// Get the user_id that owns a policy (for auth checks).
pub(crate) async fn get_policy_user(conn: &DbConn, id: i32) -> anyhow::Result<i32> {
    let sql = format!(
        "SELECT {} FROM {} WHERE {} = ?1",
        policies::USER_ID,
        policies::TABLE,
        policies::ID,
    );

    let mut result = conn.query(&sql, (id,)).await?;
    if let Some(row) = result.next().await? {
        Ok(row.get(0)?)
    } else {
        anyhow::bail!("policy not found")
    }
}

/// Hot-path: return ALL matching policies for an app, pre-sorted by priority.
/// `category_discriminants` are the integer values of the `Category` enum
/// (0=Productivity…6=Uncategorized) for the app's resolved categories.
pub(crate) async fn resolve_policies_for_app_full(
    conn: &DbConn,
    app_id: i32,
    category_discriminants: &[i32],
    uid: Uid,
    day_mask: i32,
    minute: i32,
) -> anyhow::Result<Vec<PolicyTableRow>> {
    let user_id = uid.0 as i32;

    let mut target_conditions: Vec<String> = vec![
        format!(
            "({}.{} = 0 AND {}.{} = ?)",
            policies::TABLE,
            policies::TARGET_TYPE,
            policies::TABLE,
            policies::APP_ID
        ),
        format!("{}.{} = 3", policies::TABLE, policies::TARGET_TYPE),
    ];

    if !category_discriminants.is_empty() {
        target_conditions.push(format!(
            "({}.{} = 1 AND {}.{} IN ({}))",
            policies::TABLE,
            policies::TARGET_TYPE,
            policies::TABLE,
            policies::CATEGORY,
            category_discriminants
                .iter()
                .map(|_| "?")
                .collect::<Vec<_>>()
                .join(", "),
        ));
    }

    let where_clause = format!(
        "{}.{} = ? AND ({})",
        policies::TABLE,
        policies::USER_ID,
        target_conditions.join(" OR "),
    );

    let columns = [
        policies::ID,
        policies::NAME,
        policies::PRIORITY,
        policies::EFFECT,
        policies::TARGET_TYPE,
        policies::APP_ID,
        policies::CATEGORY,
        policies::DOMAIN_PATTERN,
        policies::TIME_LIMIT_MINUTES,
        policies::USER_ID,
        policies::CREATED_BY,
    ]
    .join(", ");

    let sql = format!(
        "SELECT {columns} FROM {} WHERE {} ORDER BY {} ASC, {} ASC",
        policies::TABLE,
        where_clause,
        policies::TARGET_TYPE,
        policies::PRIORITY,
    );

    let mut params: Vec<turso::Value> = Vec::new();
    params.push(user_id.into());
    params.push(app_id.into());
    for &cat_val in category_discriminants {
        params.push(cat_val.into());
    }

    let mut result = conn.query(&sql, params_from_iter(params)).await?;
    let mut table_rows = Vec::new();
    while let Some(row) = result.next().await? {
        table_rows.push(PolicyTableRow {
            id: row.get(0)?,
            name: row.get(1)?,
            priority: row.get(2)?,
            effect: row.get(3)?,
            target_type: row.get(4)?,
            app_id: row.get::<Option<i32>>(5)?,
            category: row.get::<Option<i32>>(6)?,
            domain_pattern: row.get::<Option<String>>(7)?,
            time_limit_minutes: row.get::<Option<i32>>(8)?,
            user_id: row.get(9)?,
            created_by: row.get(10)?,
        });
    }

    if table_rows.is_empty() {
        return Ok(Vec::new());
    }

    // Load all schedule rows for these policies.
    let policy_ids: Vec<i32> = table_rows.iter().map(|r| r.id).collect();
    let schedule_rows: Vec<ScheduleRow> = load_schedules(conn, &policy_ids).await?;

    // Index schedules by policy_id.
    let schedule_map: HashMap<i32, Vec<&ScheduleRow>> = {
        let mut m: HashMap<i32, Vec<&ScheduleRow>> = HashMap::new();
        for sr in &schedule_rows {
            m.entry(sr.policy_id).or_default().push(sr);
        }
        m
    };

    // Filter by schedule and build PolicyRow.
    let mut result = Vec::new();
    for row in table_rows {
        let schedules = schedule_map.get(&row.id);

        // No schedule = always active.
        let schedule_matches = match schedules {
            None => true,
            Some(scheds) => scheds.iter().any(|s| {
                let day_match = (s.day_mask & day_mask) != 0;
                let time_match = if s.start_minute < s.end_minute {
                    minute >= s.start_minute && minute < s.end_minute
                } else {
                    minute >= s.start_minute || minute < s.end_minute
                };
                day_match && time_match
            }),
        };

        if !schedule_matches {
            continue;
        }

        result.push(row);
    }

    Ok(result)
}

/// Read all policies, optionally filtered by user_id.
pub(crate) async fn read_policies(
    conn: &DbConn,
    caller_root: bool,
    user_id: i32,
) -> anyhow::Result<Vec<PolicyTableRow>> {
    let sql = if caller_root {
        format!(
            "SELECT {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {} FROM {} ORDER BY {} ASC, {} ASC",
            policies::ID,
            policies::NAME,
            policies::PRIORITY,
            policies::EFFECT,
            policies::TARGET_TYPE,
            policies::APP_ID,
            policies::CATEGORY,
            policies::DOMAIN_PATTERN,
            policies::TIME_LIMIT_MINUTES,
            policies::USER_ID,
            policies::CREATED_BY,
            policies::TABLE,
            policies::TARGET_TYPE,
            policies::PRIORITY,
        )
    } else {
        format!(
            "SELECT {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {} FROM {} WHERE {} = ? ORDER BY {} ASC, {} ASC",
            policies::ID,
            policies::NAME,
            policies::PRIORITY,
            policies::EFFECT,
            policies::TARGET_TYPE,
            policies::APP_ID,
            policies::CATEGORY,
            policies::DOMAIN_PATTERN,
            policies::TIME_LIMIT_MINUTES,
            policies::USER_ID,
            policies::CREATED_BY,
            policies::TABLE,
            policies::USER_ID,
            policies::TARGET_TYPE,
            policies::PRIORITY,
        )
    };

    let mut result = if caller_root {
        conn.query(&sql, ()).await?
    } else {
        conn.query(&sql, (user_id,)).await?
    };

    let mut table_rows = Vec::new();
    while let Some(row) = result.next().await? {
        table_rows.push(PolicyTableRow {
            id: row.get(0)?,
            name: row.get(1)?,
            priority: row.get(2)?,
            effect: row.get(3)?,
            target_type: row.get(4)?,
            app_id: row.get::<Option<i32>>(5)?,
            category: row.get::<Option<i32>>(6)?,
            domain_pattern: row.get::<Option<String>>(7)?,
            time_limit_minutes: row.get::<Option<i32>>(8)?,
            user_id: row.get(9)?,
            created_by: row.get(10)?,
        });
    }

    Ok(table_rows)
}

/// Create a policy within a transaction (returns PolicyId).
pub(crate) async fn create_policy(
    conn: &DbConn,
    new: NewPolicy,
) -> anyhow::Result<wellbeing_core::PolicyId> {
    let sql = format!(
        "INSERT INTO {} ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10) \
         RETURNING {}",
        policies::TABLE,
        policies::NAME,
        policies::PRIORITY,
        policies::EFFECT,
        policies::TARGET_TYPE,
        policies::APP_ID,
        policies::CATEGORY,
        policies::DOMAIN_PATTERN,
        policies::TIME_LIMIT_MINUTES,
        policies::USER_ID,
        policies::CREATED_BY,
        policies::ID,
    );

    let mut result = conn
        .query(
            &sql,
            (
                new.name,
                new.priority,
                new.effect,
                new.target_type,
                new.app_id,
                new.category,
                new.domain_pattern,
                new.time_limit_minutes,
                new.user_id,
                new.created_by,
            ),
        )
        .await?;

    if let Some(row) = result.next().await? {
        Ok(wellbeing_core::PolicyId(row.get::<i32>(0)? as i64))
    } else {
        anyhow::bail!("failed to create policy")
    }
}

/// Update a policy within a transaction; returns true if a row was updated.
pub(crate) async fn update_policy(
    conn: &DbConn,
    id: i32,
    changes: UpdatePolicy,
) -> anyhow::Result<bool> {
    let mut set_clauses = Vec::new();
    let mut params: Vec<turso::Value> = Vec::new();

    if let Some(name) = changes.name {
        set_clauses.push(format!("{} = ?", policies::NAME));
        params.push(name.into());
    }
    if let Some(priority) = changes.priority {
        set_clauses.push(format!("{} = ?", policies::PRIORITY));
        params.push(priority.into());
    }
    if let Some(effect) = changes.effect {
        set_clauses.push(format!("{} = ?", policies::EFFECT));
        params.push(effect.into());
    }
    if let Some(target_type) = changes.target_type {
        set_clauses.push(format!("{} = ?", policies::TARGET_TYPE));
        params.push(target_type.into());
    }
    if let Some(app_id) = changes.app_id {
        set_clauses.push(format!("{} = ?", policies::APP_ID));
        params.push(app_id.into());
    }
    if let Some(category) = changes.category {
        set_clauses.push(format!("{} = ?", policies::CATEGORY));
        params.push(category.into());
    }
    if let Some(domain_pattern) = changes.domain_pattern {
        set_clauses.push(format!("{} = ?", policies::DOMAIN_PATTERN));
        params.push(domain_pattern.into());
    }
    if let Some(time_limit_minutes) = changes.time_limit_minutes {
        set_clauses.push(format!("{} = ?", policies::TIME_LIMIT_MINUTES));
        params.push(time_limit_minutes.into());
    }

    if set_clauses.is_empty() {
        return Ok(false);
    }

    let sql = format!(
        "UPDATE {} SET {} WHERE {} = ?",
        policies::TABLE,
        set_clauses.join(", "),
        policies::ID,
    );

    params.push(id.into());

    let rows_affected = conn.execute(&sql, params_from_iter(params)).await?;
    Ok(rows_affected > 0)
}

/// Resolve a category name to its `Category` enum discriminant.
/// Returns `None` when the name is empty or no matching category exists.
pub(crate) fn resolve_category_name_to_discriminant(name: &str) -> Option<i32> {
    if name.is_empty() {
        return None;
    }
    // Match against known Category names (case-sensitive).
    match name {
        "Productivity" => Some(0),
        "Communication" => Some(1),
        "Entertainment" => Some(2),
        "Social" => Some(3),
        "Development" => Some(4),
        "Utilities" => Some(5),
        "Uncategorized" => Some(6),
        _ => None,
    }
}

/// Delete a policy within a transaction; returns true if a row was deleted.
pub(crate) async fn delete_policy(conn: &DbConn, id: i32) -> anyhow::Result<bool> {
    let sql = format!("DELETE FROM {} WHERE {} = ?", policies::TABLE, policies::ID,);

    let rows_affected = conn.execute(&sql, (id,)).await?;
    Ok(rows_affected > 0)
}

pub(crate) async fn load_schedules(
    conn: &DbConn,
    policy_ids: &[i32],
) -> anyhow::Result<Vec<ScheduleRow>> {
    if policy_ids.is_empty() {
        return Ok(Vec::new());
    }

    let placeholders: Vec<String> = policy_ids.iter().map(|_| "?".to_string()).collect();
    let sql = format!(
        "SELECT {}, {}, {}, {} FROM {} WHERE {} IN ({})",
        policy_schedules::POLICY_ID,
        policy_schedules::START_MINUTE,
        policy_schedules::END_MINUTE,
        policy_schedules::DAY_MASK,
        policy_schedules::TABLE,
        policy_schedules::POLICY_ID,
        placeholders.join(", "),
    );

    let mut result = conn
        .query(&sql, params_from_iter(policy_ids.to_vec()))
        .await?;
    let mut rows = Vec::new();
    while let Some(row) = result.next().await? {
        rows.push(ScheduleRow {
            policy_id: row.get(0)?,
            start_minute: row.get(1)?,
            end_minute: row.get(2)?,
            day_mask: row.get(3)?,
        });
    }
    Ok(rows)
}
