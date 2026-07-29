//! Aggregated read queries for the daily_usage materialized tables.
//!
//! DAO functions only — return raw tuples via GROUP BY, no domain type
//! conversion.  Feature repositories map results to domain types.
//!
//! All queries are aggregate-only — per-row, per-date queries were removed
//! when the GUI switched to pre-aggregated summaries over D-Bus.

use wellbeing_core::Uid;

use crate::store::connection::DbConn;
use crate::store::schema_constants::{
    apps, daily_usage_by_app, daily_usage_by_category, daily_usage_by_title,
};

/// (app_class, total_millis) — aggregated per-app total across a date range.
pub type AppSummaryRow = (String, i64);

/// (app_class, title, total_millis) — aggregated per-title total across a date range.
pub type TitleSummaryRow = (String, String, i64);

/// Get today's total usage (closed + open) for a specific app.
pub(crate) async fn get_today_app_usage(
    conn: &DbConn,
    app_id: i32,
    uid: Uid,
    today: &str,
) -> anyhow::Result<i64> {
    let sql = format!(
        "SELECT {}, {} FROM {} WHERE {} = ?1 AND {} = ?2 AND {} = ?3",
        daily_usage_by_app::CLOSED_MILLIS,
        daily_usage_by_app::OPEN_MILLIS,
        daily_usage_by_app::TABLE,
        daily_usage_by_app::DATE,
        daily_usage_by_app::USER_ID,
        daily_usage_by_app::APP_ID,
    );

    let mut result = conn.query(&sql, (today, uid.0 as i32, app_id)).await?;
    if let Some(row) = result.next().await? {
        let closed: i64 = row.get(0)?;
        let open: i64 = row.get(1)?;
        Ok(closed + open)
    } else {
        Ok(0)
    }
}

/// Get per-app usage aggregated across a date range, sorted by total_millis DESC.
pub(crate) async fn app_usage_summary(
    conn: &DbConn,
    start_date: &str,
    end_date: &str,
    uid: u32,
) -> anyhow::Result<Vec<AppSummaryRow>> {
    let sql = format!(
        "SELECT a.{}, SUM(d.{}) as total \
         FROM {} d \
         INNER JOIN {} a ON a.{} = d.{} \
         WHERE d.{} >= ?1 AND d.{} <= ?2 AND d.{} = ?3 \
         GROUP BY a.{} \
         ORDER BY total DESC",
        apps::APP_CLASS,
        daily_usage_by_app::TOTAL_MILLIS,
        daily_usage_by_app::TABLE,
        apps::TABLE,
        apps::ID,
        daily_usage_by_app::APP_ID,
        daily_usage_by_app::DATE,
        daily_usage_by_app::DATE,
        daily_usage_by_app::USER_ID,
        apps::APP_CLASS,
    );

    let mut result = conn.query(&sql, (start_date, end_date, uid as i32)).await?;
    let mut rows = Vec::new();
    while let Some(row) = result.next().await? {
        rows.push((row.get(0)?, row.get(1)?));
    }
    Ok(rows)
}

/// Get per-title usage aggregated across a date range, sorted by total_millis DESC.
pub(crate) async fn title_usage_summary(
    conn: &DbConn,
    start_date: &str,
    end_date: &str,
    uid: u32,
) -> anyhow::Result<Vec<TitleSummaryRow>> {
    let sql = format!(
        "SELECT a.{}, d.{}, SUM(d.{}) as total \
         FROM {} d \
         INNER JOIN {} a ON a.{} = d.{} \
         WHERE d.{} >= ?1 AND d.{} <= ?2 AND d.{} = ?3 \
         GROUP BY a.{}, d.{} \
         ORDER BY total DESC",
        apps::APP_CLASS,
        daily_usage_by_title::TITLE,
        daily_usage_by_title::TOTAL_MILLIS,
        daily_usage_by_title::TABLE,
        apps::TABLE,
        apps::ID,
        daily_usage_by_title::APP_ID,
        daily_usage_by_title::DATE,
        daily_usage_by_title::DATE,
        daily_usage_by_title::USER_ID,
        apps::APP_CLASS,
        daily_usage_by_title::TITLE,
    );

    let mut result = conn.query(&sql, (start_date, end_date, uid as i32)).await?;
    let mut rows = Vec::new();
    while let Some(row) = result.next().await? {
        rows.push((row.get(0)?, row.get(1)?, row.get(2)?));
    }
    Ok(rows)
}

/// Get per-category usage aggregated across a date range, sorted by total_millis DESC.
/// Returns (category_discriminant, total_millis) — the category discriminant
/// is the `#[repr(u8)]` value of the [`Category`] enum (0=Productivity…6=Uncategorized).
pub(crate) async fn category_usage_summary(
    conn: &DbConn,
    start_date: &str,
    end_date: &str,
    uid: u32,
) -> anyhow::Result<Vec<(i32, i64)>> {
    let sql = format!(
        "SELECT d.{}, SUM(d.{}) as total \
         FROM {} d \
         WHERE d.{} >= ?1 AND d.{} <= ?2 AND d.{} = ?3 \
         GROUP BY d.{} \
         ORDER BY total DESC",
        daily_usage_by_category::CATEGORY,
        daily_usage_by_category::TOTAL_MILLIS,
        daily_usage_by_category::TABLE,
        daily_usage_by_category::DATE,
        daily_usage_by_category::DATE,
        daily_usage_by_category::USER_ID,
        daily_usage_by_category::CATEGORY,
    );

    let mut result = conn.query(&sql, (start_date, end_date, uid as i32)).await?;
    let mut rows = Vec::new();
    while let Some(row) = result.next().await? {
        rows.push((row.get(0)?, row.get(1)?));
    }
    Ok(rows)
}

/// Get total usage per date across a range, sorted by date ASC.
pub(crate) async fn daily_bar_totals(
    conn: &DbConn,
    start_date: &str,
    end_date: &str,
    uid: u32,
) -> anyhow::Result<Vec<(String, i64)>> {
    let sql = format!(
        "SELECT {}, SUM({}) as total \
         FROM {} \
         WHERE {} >= ?1 AND {} <= ?2 AND {} = ?3 \
         GROUP BY {} \
         ORDER BY {} ASC",
        daily_usage_by_app::DATE,
        daily_usage_by_app::TOTAL_MILLIS,
        daily_usage_by_app::TABLE,
        daily_usage_by_app::DATE,
        daily_usage_by_app::DATE,
        daily_usage_by_app::USER_ID,
        daily_usage_by_app::DATE,
        daily_usage_by_app::DATE,
    );

    let mut result = conn.query(&sql, (start_date, end_date, uid as i32)).await?;
    let mut rows = Vec::new();
    while let Some(row) = result.next().await? {
        rows.push((row.get(0)?, row.get(1)?));
    }
    Ok(rows)
}
