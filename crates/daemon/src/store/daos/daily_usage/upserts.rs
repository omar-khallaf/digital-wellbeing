//! Batch-upsert helpers for the daily_usage materialized tables.
//!
//! Every function here takes `conn: &DbConn` — they are associated
//! functions called from the delta-computation pipeline.

use wellbeing_core::{Uid, WindowTitle};

use crate::store::connection::DbConn;
use crate::store::schema_constants::{
    apps, daily_usage_by_app, daily_usage_by_category, daily_usage_by_title,
};

pub(crate) async fn upsert_closed_delta(
    conn: &DbConn,
    date: &str,
    uid: Uid,
    app_id: i32,
    delta_ms: i64,
) -> anyhow::Result<()> {
    let sql = format!(
        "INSERT INTO {} ({}, {}, {}, {}, {}) \
         VALUES (?1, ?2, ?3, ?4, 0) \
         ON CONFLICT({}, {}, {}) DO UPDATE SET {} = {} + ?4",
        daily_usage_by_app::TABLE,
        daily_usage_by_app::DATE,
        daily_usage_by_app::USER_ID,
        daily_usage_by_app::APP_ID,
        daily_usage_by_app::CLOSED_MILLIS,
        daily_usage_by_app::OPEN_MILLIS,
        daily_usage_by_app::DATE,
        daily_usage_by_app::USER_ID,
        daily_usage_by_app::APP_ID,
        daily_usage_by_app::CLOSED_MILLIS,
        daily_usage_by_app::CLOSED_MILLIS,
    );

    conn.execute(&sql, (date, uid.0 as i32, app_id, delta_ms))
        .await?;
    Ok(())
}

pub(crate) async fn upsert_open_delta(
    conn: &DbConn,
    date: &str,
    uid: Uid,
    app_id: i32,
    open_ms: i64,
) -> anyhow::Result<()> {
    let sql = format!(
        "INSERT INTO {} ({}, {}, {}, {}, {}) \
         VALUES (?1, ?2, ?3, 0, ?4) \
         ON CONFLICT({}, {}, {}) DO UPDATE SET {} = ?4",
        daily_usage_by_app::TABLE,
        daily_usage_by_app::DATE,
        daily_usage_by_app::USER_ID,
        daily_usage_by_app::APP_ID,
        daily_usage_by_app::CLOSED_MILLIS,
        daily_usage_by_app::OPEN_MILLIS,
        daily_usage_by_app::DATE,
        daily_usage_by_app::USER_ID,
        daily_usage_by_app::APP_ID,
        daily_usage_by_app::OPEN_MILLIS,
    );

    conn.execute(&sql, (date, uid.0 as i32, app_id, open_ms))
        .await?;
    Ok(())
}

pub(crate) async fn upsert_closed_delta_by_title(
    conn: &DbConn,
    date: &str,
    uid: Uid,
    app_class: &str,
    title: &WindowTitle,
    delta_ms: i64,
) -> anyhow::Result<()> {
    let app_id: i32 = match conn
        .query(
            &format!(
                "SELECT {} FROM {} WHERE {} = ?1",
                apps::ID,
                apps::TABLE,
                apps::APP_CLASS
            ),
            (app_class,),
        )
        .await
    {
        Ok(mut result) => {
            if let Some(row) = result.next().await.ok().flatten() {
                row.get(0)?
            } else {
                anyhow::bail!("app not found")
            }
        }
        Err(e) => return Err(anyhow::anyhow!("query failed: {}", e)),
    };

    let sql = format!(
        "INSERT INTO {} ({}, {}, {}, {}, {}, {}) \
         VALUES (?1, ?2, ?3, ?4, ?5, 0) \
         ON CONFLICT({}, {}, {}, {}) DO UPDATE SET {} = {} + ?5",
        daily_usage_by_title::TABLE,
        daily_usage_by_title::DATE,
        daily_usage_by_title::USER_ID,
        daily_usage_by_title::APP_ID,
        daily_usage_by_title::TITLE,
        daily_usage_by_title::CLOSED_MILLIS,
        daily_usage_by_title::OPEN_MILLIS,
        daily_usage_by_title::DATE,
        daily_usage_by_title::USER_ID,
        daily_usage_by_title::APP_ID,
        daily_usage_by_title::TITLE,
        daily_usage_by_title::CLOSED_MILLIS,
        daily_usage_by_title::CLOSED_MILLIS,
    );

    conn.execute(&sql, (date, uid.0 as i32, app_id, title.as_str(), delta_ms))
        .await?;
    Ok(())
}

pub(crate) async fn upsert_open_delta_by_title(
    conn: &DbConn,
    date: &str,
    uid: Uid,
    app_class: &str,
    title: &WindowTitle,
    open_ms: i64,
) -> anyhow::Result<()> {
    let app_id: i32 = match conn
        .query(
            &format!(
                "SELECT {} FROM {} WHERE {} = ?1",
                apps::ID,
                apps::TABLE,
                apps::APP_CLASS
            ),
            (app_class,),
        )
        .await
    {
        Ok(mut result) => {
            if let Some(row) = result.next().await.ok().flatten() {
                row.get(0)?
            } else {
                anyhow::bail!("app not found")
            }
        }
        Err(e) => return Err(anyhow::anyhow!("query failed: {}", e)),
    };

    let sql = format!(
        "INSERT INTO {} ({}, {}, {}, {}, {}, {}) \
         VALUES (?1, ?2, ?3, ?4, 0, ?5) \
         ON CONFLICT({}, {}, {}, {}) DO UPDATE SET {} = ?5",
        daily_usage_by_title::TABLE,
        daily_usage_by_title::DATE,
        daily_usage_by_title::USER_ID,
        daily_usage_by_title::APP_ID,
        daily_usage_by_title::TITLE,
        daily_usage_by_title::CLOSED_MILLIS,
        daily_usage_by_title::OPEN_MILLIS,
        daily_usage_by_title::DATE,
        daily_usage_by_title::USER_ID,
        daily_usage_by_title::APP_ID,
        daily_usage_by_title::TITLE,
        daily_usage_by_title::OPEN_MILLIS,
    );

    conn.execute(&sql, (date, uid.0 as i32, app_id, title.as_str(), open_ms))
        .await?;
    Ok(())
}

pub(crate) async fn upsert_closed_delta_by_category(
    conn: &DbConn,
    date: &str,
    uid: Uid,
    category_id: i32,
    delta_ms: i64,
) -> anyhow::Result<()> {
    let sql = format!(
        "INSERT INTO {} ({}, {}, {}, {}, {}) \
         VALUES (?1, ?2, ?3, ?4, 0) \
         ON CONFLICT({}, {}, {}) DO UPDATE SET {} = {} + ?4",
        daily_usage_by_category::TABLE,
        daily_usage_by_category::DATE,
        daily_usage_by_category::USER_ID,
        daily_usage_by_category::CATEGORY,
        daily_usage_by_category::CLOSED_MILLIS,
        daily_usage_by_category::OPEN_MILLIS,
        daily_usage_by_category::DATE,
        daily_usage_by_category::USER_ID,
        daily_usage_by_category::CATEGORY,
        daily_usage_by_category::CLOSED_MILLIS,
        daily_usage_by_category::CLOSED_MILLIS,
    );

    conn.execute(&sql, (date, uid.0 as i32, category_id, delta_ms))
        .await?;
    Ok(())
}

pub(crate) async fn upsert_open_delta_by_category(
    conn: &DbConn,
    date: &str,
    uid: Uid,
    category_id: i32,
    open_ms: i64,
) -> anyhow::Result<()> {
    let sql = format!(
        "INSERT INTO {} ({}, {}, {}, {}, {}) \
         VALUES (?1, ?2, ?3, 0, ?4) \
         ON CONFLICT({}, {}, {}) DO UPDATE SET {} = ?4",
        daily_usage_by_category::TABLE,
        daily_usage_by_category::DATE,
        daily_usage_by_category::USER_ID,
        daily_usage_by_category::CATEGORY,
        daily_usage_by_category::CLOSED_MILLIS,
        daily_usage_by_category::OPEN_MILLIS,
        daily_usage_by_category::DATE,
        daily_usage_by_category::USER_ID,
        daily_usage_by_category::CATEGORY,
        daily_usage_by_category::OPEN_MILLIS,
    );

    conn.execute(&sql, (date, uid.0 as i32, category_id, open_ms))
        .await?;
    Ok(())
}
