//! Internal query helpers for the policy repository (take an explicit
//! `&mut DbConn` argument so the transaction methods can share a connection).

use std::collections::HashMap;

use diesel::{BoolExpressionMethods, ExpressionMethods, QueryDsl, SelectableHelper};
use diesel_async::RunQueryDsl;
use wellbeing_core::{TargetType, Uid};

use crate::policy::data::insert::{NewPolicy, UpdatePolicy};
use crate::policy::data::models::{PolicyRow, PolicyTableRow, ScheduleRow};
use crate::store::connection::DbConn;
use crate::store::schema;

/// Get the user_id that owns a policy (for auth checks).
pub(crate) async fn get_policy_user(conn: &mut DbConn, id: i32) -> anyhow::Result<i32> {
    use schema::policies::columns as c;
    let user_id: i32 = schema::policies::table
        .filter(c::id.eq(id))
        .select(c::user_id)
        .first(conn)
        .await?;
    Ok(user_id)
}

/// Hot-path: return ALL matching policies for an app, pre-sorted by priority.
///
/// Uses pure diesel DSL for the query + Rust-side schedule filtering
/// (bitwise day_mask and midnight-wrapping time ranges cannot be expressed
/// in diesel DSL). Category names are resolved separately for the
/// category-targeted policies.
pub(crate) async fn resolve_policies_for_app_full(
    conn: &mut DbConn,
    app_id: i32,
    category_names: &[String],
    uid: Uid,
    day_mask: i32,
    minute: i32,
) -> anyhow::Result<Vec<PolicyRow>> {
    use schema::categories::columns as ca;
    use schema::policies::columns as p;

    let user_id = uid.0 as i32;

    // Resolve category IDs from names.
    let cat_ids: Vec<i32> = if category_names.is_empty() {
        Vec::new()
    } else {
        schema::categories::table
            .filter(ca::name.eq_any(category_names))
            .select(ca::id)
            .load(conn)
            .await?
    };

    // Build target-matching query: Any (3) OR App-specific (0) OR Category-match (1).
    let mut query = schema::policies::table
        .filter(p::user_id.eq(user_id))
        .into_boxed();

    let any_target = p::target_type.eq(TargetType::Any as i32);
    let app_target = p::target_type
        .eq(TargetType::App as i32)
        .and(p::app_id.eq(app_id));

    query = if cat_ids.is_empty() {
        query.filter(any_target.or(app_target))
    } else {
        let cat_target = p::target_type
            .eq(TargetType::Category as i32)
            .and(p::category_id.eq_any(&cat_ids));
        query.filter(any_target.or(app_target).or(cat_target))
    };

    // Load matching policy table rows.
    let table_rows: Vec<PolicyTableRow> = query
        .select(PolicyTableRow::as_select())
        .order((p::target_type.asc(), p::priority.asc()))
        .load(conn)
        .await?;

    if table_rows.is_empty() {
        return Ok(Vec::new());
    }

    // Load all schedule rows for these policies.
    let policy_ids: Vec<i32> = table_rows.iter().map(|r| r.id).collect();
    let schedule_rows: Vec<ScheduleRow> = schema::policy_schedules::table
        .filter(schema::policy_schedules::policy_id.eq_any(&policy_ids))
        .load(conn)
        .await?;

    // Index schedules by policy_id.
    let schedule_map: HashMap<i32, Vec<&ScheduleRow>> = {
        let mut m: HashMap<i32, Vec<&ScheduleRow>> = HashMap::new();
        for sr in &schedule_rows {
            m.entry(sr.policy_id).or_default().push(sr);
        }
        m
    };

    // Resolve category id→name map for category-targeted policies.
    let cat_id_to_name: HashMap<i32, String> = {
        let cat_ids_with_policy: Vec<i32> = table_rows
            .iter()
            .filter(|r| r.target_type == TargetType::Category as i32)
            .filter_map(|r| r.category_id)
            .collect();
        if cat_ids_with_policy.is_empty() {
            HashMap::new()
        } else {
            let names: Vec<(i32, String)> = schema::categories::table
                .filter(ca::id.eq_any(&cat_ids_with_policy))
                .select((ca::id, ca::name))
                .load(conn)
                .await?;
            names.into_iter().collect()
        }
    };

    // Filter by schedule and build PolicyRow.
    let mut result: Vec<PolicyRow> = Vec::new();
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

        let category_name = if row.target_type == TargetType::Category as i32 {
            row.category_id
                .and_then(|cid| cat_id_to_name.get(&cid).cloned())
        } else {
            None
        };

        result.push(PolicyRow {
            id: row.id,
            name: row.name,
            priority: row.priority,
            effect: row.effect,
            target_type: row.target_type,
            app_id: row.app_id,
            category_id: row.category_id,
            domain_pattern: row.domain_pattern,
            time_limit_minutes: row.time_limit_minutes,
            user_id: row.user_id,
            created_by: row.created_by,
            created_at: row.created_at,
            updated_at: row.updated_at,
            category_name,
        });
    }

    Ok(result)
}

/// Read all policies, optionally filtered by user_id.
pub(crate) async fn read_policies(
    conn: &mut DbConn,
    caller_root: bool,
    user_id: i32,
) -> anyhow::Result<Vec<PolicyRow>> {
    use schema::policies::columns as p;
    let table_rows = if caller_root {
        schema::policies::table
            .order((p::target_type.asc(), p::priority.asc()))
            .select(PolicyTableRow::as_select())
            .load::<PolicyTableRow>(conn)
            .await?
    } else {
        schema::policies::table
            .filter(p::user_id.eq(user_id))
            .order((p::target_type.asc(), p::priority.asc()))
            .select(PolicyTableRow::as_select())
            .load::<PolicyTableRow>(conn)
            .await?
    };
    Ok(table_rows.into_iter().map(PolicyRow::from).collect())
}

/// Create a policy within a transaction (returns PolicyId).
pub(crate) async fn create_policy(
    conn: &mut DbConn,
    new: NewPolicy,
) -> anyhow::Result<wellbeing_core::PolicyId> {
    let row: PolicyTableRow = diesel::insert_into(schema::policies::table)
        .values(&new)
        .returning(PolicyTableRow::as_returning())
        .get_result(conn)
        .await?;
    Ok(wellbeing_core::PolicyId(row.id as i64))
}

/// Update a policy within a transaction; returns true if a row was updated.
pub(crate) async fn update_policy(
    conn: &mut DbConn,
    id: i32,
    changes: UpdatePolicy,
) -> anyhow::Result<bool> {
    use schema::policies::columns as p;
    let rows = diesel::update(schema::policies::table.filter(p::id.eq(id)))
        .set(&changes)
        .execute(conn)
        .await?;
    Ok(rows > 0)
}

/// Resolve a category name to its row id. Returns `None` when the name
/// is empty or no matching category exists.
pub(crate) async fn resolve_category_name(
    conn: &mut DbConn,
    name: &str,
) -> anyhow::Result<Option<i32>> {
    use schema::categories::columns as c;
    if name.is_empty() {
        return Ok(None);
    }
    Ok(schema::categories::table
        .filter(c::name.eq(name))
        .select(c::id)
        .first(conn)
        .await
        .ok())
}

/// Delete a policy within a transaction; returns true if a row was deleted.
pub(crate) async fn delete_policy(conn: &mut DbConn, id: i32) -> anyhow::Result<bool> {
    use schema::policies::columns as p;
    let rows = diesel::delete(schema::policies::table.filter(p::id.eq(id)))
        .execute(conn)
        .await?;
    Ok(rows > 0)
}
