//! App registry repository — the `apps` table.
//!
//! Uses a transactional get-or-insert pattern: SELECT first, then INSERT
//! with RETURNING if not found. No ON CONFLICT.

use wellbeing_core::AppClass;

use crate::store::connection::DbConn;
use crate::store::schema_constants::apps;

/// App repository — provides static methods.
pub(crate) struct AppsDao;

impl AppsDao {
    /// Ensure an app exists using an existing connection (for use inside
    /// larger transactions).
    pub(crate) async fn ensure_app(conn: &DbConn, app_class: &AppClass) -> anyhow::Result<i32> {
        // Try to find existing app
        let sql = format!(
            "SELECT {} FROM {} WHERE {} = ?1",
            apps::ID,
            apps::TABLE,
            apps::APP_CLASS,
        );

        let mut result = conn.query(&sql, (app_class.as_ref(),)).await?;
        if let Some(row) = result.next().await? {
            let id: i32 = row.get(0)?;
            return Ok(id);
        }

        // Insert new app and return the generated id
        let insert_sql = format!(
            "INSERT INTO {} ({}) VALUES (?1) RETURNING {}",
            apps::TABLE,
            apps::APP_CLASS,
            apps::ID,
        );

        let mut result = conn.query(&insert_sql, (app_class.as_ref(),)).await?;
        if let Some(row) = result.next().await? {
            let id: i32 = row.get(0)?;
            Ok(id)
        } else {
            anyhow::bail!("failed to insert app")
        }
    }

    /// Batch-load app ids to `app_class` mappings for the provided ids.
    pub(crate) async fn batch_load_apps(
        conn: &DbConn,
        app_ids: &[i32],
    ) -> anyhow::Result<std::collections::HashMap<i32, String>> {
        if app_ids.is_empty() {
            return Ok(std::collections::HashMap::new());
        }

        let placeholders: Vec<String> = app_ids.iter().map(|_| "?".to_string()).collect();
        let sql = format!(
            "SELECT {}, {} FROM {} WHERE {} IN ({})",
            apps::ID,
            apps::APP_CLASS,
            apps::TABLE,
            apps::ID,
            placeholders.join(", "),
        );

        let mut result = conn
            .query(&sql, turso::params_from_iter(app_ids.to_vec()))
            .await?;
        let mut map = std::collections::HashMap::new();
        while let Some(row) = result.next().await? {
            map.insert(row.get::<i32>(0)?, row.get::<String>(1)?);
        }
        Ok(map)
    }
}
