//! Batch-upsert helpers for the daily_usage materialized tables.
//!
//! Every function takes `conn: &DbConn` and a slice of typed items,
//! then issues a single multi-row INSERT … ON CONFLICT … DO UPDATE SET
//! statement.  N individual round-trips become 1.

use turso::{Value, params_from_iter};
use wellbeing_core::{Uid, WindowTitle};

use crate::store::connection::DbConn;
use crate::store::schema_constants::{
    daily_usage_by_app, daily_usage_by_category, daily_usage_by_title,
};

// ---------------------------------------------------------------------------
// Item types — one per table × open/closed pair
// ---------------------------------------------------------------------------

pub(crate) struct ClosedAppItem {
    pub date: String,
    pub uid: Uid,
    pub app_id: i32,
    pub delta_ms: i64,
}

pub(crate) struct ClosedTitleItem {
    pub date: String,
    pub uid: Uid,
    pub app_id: i32,
    pub title: WindowTitle,
    pub delta_ms: i64,
}

pub(crate) struct ClosedCategoryItem {
    pub date: String,
    pub uid: Uid,
    pub category_id: i32,
    pub delta_ms: i64,
}

pub(crate) struct OpenAppItem {
    pub date: String,
    pub uid: Uid,
    pub app_id: i32,
    pub open_ms: i64,
}

pub(crate) struct OpenTitleItem {
    pub date: String,
    pub uid: Uid,
    pub app_id: i32,
    pub title: WindowTitle,
    pub open_ms: i64,
}

pub(crate) struct OpenCategoryItem {
    pub date: String,
    pub uid: Uid,
    pub category_id: i32,
    pub open_ms: i64,
}

// ---------------------------------------------------------------------------
// Core batch executor
// ---------------------------------------------------------------------------

/// Execute a multi-row INSERT … ON CONFLICT … DO UPDATE SET.
///
/// Builds the VALUES clause dynamically from `params.len() / n_cols`.
/// Each entry in `set_exprs` is a literal SQL fragment placed directly
/// in the SET clause (e.g. `"col=col+excluded.col"` or `"col=0"`).
async fn exec_batch(
    conn: &DbConn,
    table: &str,
    insert_cols: &[&str],
    pk_cols: &[&str],
    set_exprs: &[&str],
    n_cols: usize,
    params: Vec<Value>,
) -> anyhow::Result<()> {
    let n = params.len() / n_cols;
    debug_assert_eq!(params.len() % n_cols, 0);

    let cols = insert_cols.join(",");
    let pk = pk_cols.join(",");

    // VALUES (?,?,…), (?,?,…), …
    let row = format!("({})", vec!["?"; n_cols].join(","));
    let mut values = String::with_capacity(n * row.len() + n.saturating_sub(1) * 2);
    for i in 0..n {
        if i > 0 {
            values.push(',');
        }
        values.push_str(&row);
    }

    let sql = format!(
        "INSERT INTO {table}({cols})VALUES{values}ON CONFLICT({pk})DO UPDATE SET {}",
        set_exprs.join(","),
    );

    conn.execute(&sql, params_from_iter(params)).await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// daily_usage_by_app
// ---------------------------------------------------------------------------

pub(crate) async fn batch_upsert_closed_apps(
    conn: &DbConn,
    items: &[ClosedAppItem],
) -> anyhow::Result<()> {
    if items.is_empty() {
        return Ok(());
    }
    let mut p: Vec<Value> = Vec::with_capacity(items.len() * 5);
    for it in items {
        p.push(it.date.clone().into());
        p.push((it.uid.0 as i32).into());
        p.push(it.app_id.into());
        p.push(it.delta_ms.into());
        p.push(Value::Integer(0));
    }
    exec_batch(
        conn,
        daily_usage_by_app::TABLE,
        &[
            daily_usage_by_app::DATE,
            daily_usage_by_app::USER_ID,
            daily_usage_by_app::APP_ID,
            daily_usage_by_app::CLOSED_MILLIS,
            daily_usage_by_app::OPEN_MILLIS,
        ],
        &[
            daily_usage_by_app::DATE,
            daily_usage_by_app::USER_ID,
            daily_usage_by_app::APP_ID,
        ],
        &[
            &format!(
                "{cl}={cl}+excluded.{cl}",
                cl = daily_usage_by_app::CLOSED_MILLIS
            ),
            &format!("{}=0", daily_usage_by_app::OPEN_MILLIS),
        ],
        5,
        p,
    )
    .await
}

pub(crate) async fn batch_upsert_open_apps(
    conn: &DbConn,
    items: &[OpenAppItem],
) -> anyhow::Result<()> {
    if items.is_empty() {
        return Ok(());
    }
    let mut p: Vec<Value> = Vec::with_capacity(items.len() * 5);
    for it in items {
        p.push(it.date.clone().into());
        p.push((it.uid.0 as i32).into());
        p.push(it.app_id.into());
        p.push(Value::Integer(0));
        p.push(it.open_ms.into());
    }
    exec_batch(
        conn,
        daily_usage_by_app::TABLE,
        &[
            daily_usage_by_app::DATE,
            daily_usage_by_app::USER_ID,
            daily_usage_by_app::APP_ID,
            daily_usage_by_app::CLOSED_MILLIS,
            daily_usage_by_app::OPEN_MILLIS,
        ],
        &[
            daily_usage_by_app::DATE,
            daily_usage_by_app::USER_ID,
            daily_usage_by_app::APP_ID,
        ],
        &[&format!(
            "{}=excluded.{}",
            daily_usage_by_app::OPEN_MILLIS,
            daily_usage_by_app::OPEN_MILLIS
        )],
        5,
        p,
    )
    .await
}

// ---------------------------------------------------------------------------
// daily_usage_by_title

pub(crate) async fn batch_upsert_closed_titles(
    conn: &DbConn,
    items: &[ClosedTitleItem],
) -> anyhow::Result<()> {
    if items.is_empty() {
        return Ok(());
    }
    let mut p: Vec<Value> = Vec::with_capacity(items.len() * 6);
    for it in items {
        p.push(it.date.clone().into());
        p.push((it.uid.0 as i32).into());
        p.push(it.app_id.into());
        p.push(it.title.as_str().to_string().into());
        p.push(it.delta_ms.into());
        p.push(Value::Integer(0));
    }
    exec_batch(
        conn,
        daily_usage_by_title::TABLE,
        &[
            daily_usage_by_title::DATE,
            daily_usage_by_title::USER_ID,
            daily_usage_by_title::APP_ID,
            daily_usage_by_title::TITLE,
            daily_usage_by_title::CLOSED_MILLIS,
            daily_usage_by_title::OPEN_MILLIS,
        ],
        &[
            daily_usage_by_title::DATE,
            daily_usage_by_title::USER_ID,
            daily_usage_by_title::APP_ID,
            daily_usage_by_title::TITLE,
        ],
        &[
            &format!(
                "{cl}={cl}+excluded.{cl}",
                cl = daily_usage_by_title::CLOSED_MILLIS
            ),
            &format!("{}=0", daily_usage_by_title::OPEN_MILLIS),
        ],
        6,
        p,
    )
    .await
}

pub(crate) async fn batch_upsert_open_titles(
    conn: &DbConn,
    items: &[OpenTitleItem],
) -> anyhow::Result<()> {
    if items.is_empty() {
        return Ok(());
    }
    let mut p: Vec<Value> = Vec::with_capacity(items.len() * 6);
    for it in items {
        p.push(it.date.clone().into());
        p.push((it.uid.0 as i32).into());
        p.push(it.app_id.into());
        p.push(it.title.as_str().to_string().into());
        p.push(Value::Integer(0));
        p.push(it.open_ms.into());
    }
    exec_batch(
        conn,
        daily_usage_by_title::TABLE,
        &[
            daily_usage_by_title::DATE,
            daily_usage_by_title::USER_ID,
            daily_usage_by_title::APP_ID,
            daily_usage_by_title::TITLE,
            daily_usage_by_title::CLOSED_MILLIS,
            daily_usage_by_title::OPEN_MILLIS,
        ],
        &[
            daily_usage_by_title::DATE,
            daily_usage_by_title::USER_ID,
            daily_usage_by_title::APP_ID,
            daily_usage_by_title::TITLE,
        ],
        &[&format!(
            "{}=excluded.{}",
            daily_usage_by_title::OPEN_MILLIS,
            daily_usage_by_title::OPEN_MILLIS
        )],
        6,
        p,
    )
    .await
}

// ---------------------------------------------------------------------------
// daily_usage_by_category

pub(crate) async fn batch_upsert_closed_categories(
    conn: &DbConn,
    items: &[ClosedCategoryItem],
) -> anyhow::Result<()> {
    if items.is_empty() {
        return Ok(());
    }
    let mut p: Vec<Value> = Vec::with_capacity(items.len() * 5);
    for it in items {
        p.push(it.date.clone().into());
        p.push((it.uid.0 as i32).into());
        p.push(it.category_id.into());
        p.push(it.delta_ms.into());
        p.push(Value::Integer(0));
    }
    exec_batch(
        conn,
        daily_usage_by_category::TABLE,
        &[
            daily_usage_by_category::DATE,
            daily_usage_by_category::USER_ID,
            daily_usage_by_category::CATEGORY,
            daily_usage_by_category::CLOSED_MILLIS,
            daily_usage_by_category::OPEN_MILLIS,
        ],
        &[
            daily_usage_by_category::DATE,
            daily_usage_by_category::USER_ID,
            daily_usage_by_category::CATEGORY,
        ],
        &[
            &format!(
                "{cl}={cl}+excluded.{cl}",
                cl = daily_usage_by_category::CLOSED_MILLIS
            ),
            &format!("{}=0", daily_usage_by_category::OPEN_MILLIS),
        ],
        5,
        p,
    )
    .await
}

pub(crate) async fn batch_upsert_open_categories(
    conn: &DbConn,
    items: &[OpenCategoryItem],
) -> anyhow::Result<()> {
    if items.is_empty() {
        return Ok(());
    }
    let mut p: Vec<Value> = Vec::with_capacity(items.len() * 5);
    for it in items {
        p.push(it.date.clone().into());
        p.push((it.uid.0 as i32).into());
        p.push(it.category_id.into());
        p.push(Value::Integer(0));
        p.push(it.open_ms.into());
    }
    exec_batch(
        conn,
        daily_usage_by_category::TABLE,
        &[
            daily_usage_by_category::DATE,
            daily_usage_by_category::USER_ID,
            daily_usage_by_category::CATEGORY,
            daily_usage_by_category::CLOSED_MILLIS,
            daily_usage_by_category::OPEN_MILLIS,
        ],
        &[
            daily_usage_by_category::DATE,
            daily_usage_by_category::USER_ID,
            daily_usage_by_category::CATEGORY,
        ],
        &[&format!(
            "{}=excluded.{}",
            daily_usage_by_category::OPEN_MILLIS,
            daily_usage_by_category::OPEN_MILLIS
        )],
        5,
        p,
    )
    .await
}
