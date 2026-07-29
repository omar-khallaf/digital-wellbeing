//! Categories repository — the `app_categories` table only.
//!
//! The old `categories` table has been removed — categories are now a
//! fixed `#[repr(u8)]` enum in `wellbeing_core::Category`.  Only the
//! `app_categories` table (user-specific category overrides) remains.

use std::collections::HashMap;

use turso::params_from_iter;
use wellbeing_core::Uid;
use wellbeing_core::{AppCategoryRow, AppClass, Category};

use crate::store::connection::DbConn;
use crate::store::schema_constants::{app_categories, apps};

/// System-level user id sentinel (root / no specific user).
const SYSTEM_USER_ID: i32 = 0;

/// Default category value stored in the database.
const DEFAULT_CATEGORY_INT: i32 = Category::Uncategorized as i32;

/// Category repository — provides static methods.
pub(crate) struct CategoryDao;

impl CategoryDao {
    /// Resolve category **ints** for a batch of (uid, app_id) pairs.
    /// User-specific entries take precedence; system-global (user_id = 0)
    /// entries are fallback. Unmapped apps get `Uncategorized` (6).
    ///
    /// Batched: one query per distinct UID + one system-global query,
    /// instead of 2N round-trips (was 2 per pair).
    pub(crate) async fn resolve_category_map(
        conn: &DbConn,
        pairs: &[(Uid, i32)],
    ) -> HashMap<(Uid, i32), Vec<i32>> {
        let mut map: HashMap<(Uid, i32), Vec<i32>> = HashMap::new();
        if pairs.is_empty() {
            return map;
        }

        // Group pairs by UID so we can batch per-UID queries
        let mut uid_groups: std::collections::HashMap<u32, Vec<i32>> =
            std::collections::HashMap::new();
        let mut all_app_set: std::collections::BTreeSet<i32> = std::collections::BTreeSet::new();
        for &(uid, app_id) in pairs {
            uid_groups.entry(uid.0).or_default().push(app_id);
            all_app_set.insert(app_id);
        }
        let all_app_ids: Vec<i32> = all_app_set.into_iter().collect();

        // Batch 1: user-specific lookups — one query per distinct uid
        let mut user_map: HashMap<(Uid, i32), Vec<i32>> = HashMap::new();
        for (&raw_uid, app_ids) in &uid_groups {
            let app_ph: Vec<String> = app_ids.iter().map(|_| "?".to_string()).collect();
            let sql = format!(
                "SELECT {}, {} FROM {} WHERE {} IN ({}) AND {} = ? AND {} = 0 AND {} IS NOT NULL",
                app_categories::APP_ID,
                app_categories::CATEGORY,
                app_categories::TABLE,
                app_categories::APP_ID,
                app_ph.join(", "),
                app_categories::USER_ID,
                app_categories::IGNORE,
                app_categories::CATEGORY,
            );
            let mut params: Vec<turso::Value> = app_ids.iter().map(|&id| id.into()).collect();
            params.push((raw_uid as i32).into());

            if let Ok(mut result) = conn.query(&sql, params_from_iter(params)).await {
                while let Some(row) = result.next().await.ok().flatten() {
                    if let (Ok(app_id_val), Ok(cat_val)) = (row.get::<i32>(0), row.get::<i32>(1)) {
                        user_map
                            .entry((Uid(raw_uid), app_id_val))
                            .or_default()
                            .push(cat_val);
                    }
                }
            }
        }

        // Batch 2: system-global lookup (user_id = 0)
        let mut sys_map: HashMap<i32, Vec<i32>> = HashMap::new();
        let all_app_ph: Vec<String> = all_app_ids.iter().map(|_| "?".to_string()).collect();
        let sys_sql = format!(
            "SELECT {}, {} FROM {} WHERE {} IN ({}) AND {} = 0 AND {} = 0 AND {} IS NOT NULL",
            app_categories::APP_ID,
            app_categories::CATEGORY,
            app_categories::TABLE,
            app_categories::APP_ID,
            all_app_ph.join(", "),
            app_categories::USER_ID,
            app_categories::IGNORE,
            app_categories::CATEGORY,
        );
        if let Ok(mut result) = conn.query(&sys_sql, params_from_iter(all_app_ids)).await {
            while let Some(row) = result.next().await.ok().flatten() {
                if let (Ok(app_id_val), Ok(cat_val)) = (row.get::<i32>(0), row.get::<i32>(1)) {
                    sys_map.entry(app_id_val).or_default().push(cat_val);
                }
            }
        }

        // Populate results: user-specific → system-global → default
        for &(uid, app_id) in pairs {
            let ids = if let Some(cats) = user_map.get(&(uid, app_id)) {
                cats.clone()
            } else if let Some(cats) = sys_map.get(&app_id) {
                cats.clone()
            } else {
                vec![DEFAULT_CATEGORY_INT]
            };
            map.insert((uid, app_id), ids);
        }

        map
    }

    /// Resolve categories for an app (user-specific then system fallback).
    /// Returns `Category` enum values from the stored integer.
    pub(crate) async fn resolve_categories_for_app(
        conn: &DbConn,
        app_class: &AppClass,
        uid: Uid,
    ) -> anyhow::Result<Vec<Category>> {
        // Try user-specific categories first
        let cat_ints: Vec<i32> = match conn
            .query(
                &format!(
                    "SELECT ac.{} FROM {} ac \
                     INNER JOIN {} a ON a.{} = ac.{} \
                     WHERE a.{} = ?1 AND ac.{} IS NOT NULL AND ac.{} = 0 AND ac.{} = ?2",
                    app_categories::CATEGORY,
                    app_categories::TABLE,
                    apps::TABLE,
                    apps::ID,
                    app_categories::APP_ID,
                    apps::APP_CLASS,
                    app_categories::CATEGORY,
                    app_categories::IGNORE,
                    app_categories::USER_ID,
                ),
                (app_class.as_ref(), uid.0 as i32),
            )
            .await
        {
            Ok(mut result) => {
                let mut ids = Vec::new();
                while let Some(row) = result.next().await.ok().flatten() {
                    if let Ok(cat_val) = row.get::<i32>(0) {
                        ids.push(cat_val);
                    }
                }
                ids
            }
            Err(e) => return Err(anyhow::anyhow!("query failed: {}", e)),
        };

        if !cat_ints.is_empty() {
            return Ok(cat_ints
                .into_iter()
                .filter_map(|v| Category::try_from(v).ok())
                .collect());
        }

        // System-global fallback (user_id = 0)
        let fallback_ints: Vec<i32> = match conn
            .query(
                &format!(
                    "SELECT ac.{} FROM {} ac \
                     INNER JOIN {} a ON a.{} = ac.{} \
                     WHERE a.{} = ?1 AND ac.{} IS NOT NULL AND ac.{} = 0 AND ac.{} = ?2",
                    app_categories::CATEGORY,
                    app_categories::TABLE,
                    apps::TABLE,
                    apps::ID,
                    app_categories::APP_ID,
                    apps::APP_CLASS,
                    app_categories::CATEGORY,
                    app_categories::IGNORE,
                    app_categories::USER_ID,
                ),
                (app_class.as_ref(), SYSTEM_USER_ID),
            )
            .await
        {
            Ok(mut result) => {
                let mut ids = Vec::new();
                while let Some(row) = result.next().await.ok().flatten() {
                    if let Ok(cat_val) = row.get::<i32>(0) {
                        ids.push(cat_val);
                    }
                }
                ids
            }
            Err(e) => return Err(anyhow::anyhow!("query failed: {}", e)),
        };

        Ok(fallback_ints
            .into_iter()
            .filter_map(|v| Category::try_from(v).ok())
            .collect())
    }

    /// Get app_categories for a user (user-specific overrides).
    pub(crate) async fn list_app_categories(
        conn: &DbConn,
        caller: u32,
    ) -> anyhow::Result<Vec<AppCategoryRow>> {
        let sql = format!(
            "SELECT a.{}, ac.{}, ac.{}, ac.{}, ac.{}, ac.{} \
             FROM {} ac \
             INNER JOIN {} a ON a.{} = ac.{} \
             WHERE ac.{} = ?1",
            apps::APP_CLASS,
            app_categories::USER_ID,
            app_categories::CATEGORY,
            app_categories::DISPLAY_NAME,
            app_categories::ICON_PATH,
            app_categories::IGNORE,
            app_categories::TABLE,
            apps::TABLE,
            apps::ID,
            app_categories::APP_ID,
            app_categories::USER_ID,
        );

        let mut result = conn.query(&sql, (caller as i32,)).await?;
        let mut rows = Vec::new();
        while let Some(row) = result.next().await? {
            let cat_int: i32 = row.get::<Option<i32>>(2)?.unwrap_or(DEFAULT_CATEGORY_INT);
            rows.push(AppCategoryRow {
                app_class: AppClass::new(&row.get::<String>(0)?)
                    .unwrap_or_else(|_| AppClass::new("?").unwrap()),
                user_id: Uid(row.get::<i32>(1)? as u32),
                category: Category::try_from(cat_int).unwrap_or(Category::Uncategorized),
                display_name: row.get::<Option<String>>(3)?.unwrap_or_default(),
                icon_path: row.get::<Option<String>>(4)?.unwrap_or_default(),
                ignore: row.get(5)?,
            });
        }
        Ok(rows)
    }

    /// Set or create an app category override (upsert).
    pub(crate) async fn upsert_app_category(
        conn: &DbConn,
        app_class: String,
        category: Category,
        caller: u32,
        now_str: &str,
    ) -> anyhow::Result<()> {
        let uid = caller as i32;
        let cat_val: Option<i32> = Some(category as i32);

        // Resolve app_id
        let app_id: i32 = match conn
            .query(
                &format!(
                    "SELECT {} FROM {} WHERE {} = ?1",
                    apps::ID,
                    apps::TABLE,
                    apps::APP_CLASS
                ),
                (app_class.as_str(),),
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

        // Try to update existing row
        let update_sql = format!(
            "UPDATE {} SET {} = ?1, {} = ?2 WHERE {} = ?3 AND {} = ?4",
            app_categories::TABLE,
            app_categories::CATEGORY,
            app_categories::UPDATED_AT,
            app_categories::APP_ID,
            app_categories::USER_ID,
        );

        let rows_affected = conn
            .execute(&update_sql, (cat_val, now_str, app_id, uid))
            .await?;

        if rows_affected == 0 {
            // Insert new row
            let insert_sql = format!(
                "INSERT INTO {} ({}, {}, {}, {}, {}, {}, {}) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                app_categories::TABLE,
                app_categories::APP_ID,
                app_categories::USER_ID,
                app_categories::CATEGORY,
                app_categories::DISPLAY_NAME,
                app_categories::ICON_PATH,
                app_categories::IGNORE,
                app_categories::UPDATED_AT,
            );

            conn.execute(
                &insert_sql,
                (
                    app_id,
                    uid,
                    cat_val,
                    None::<String>,
                    None::<String>,
                    false,
                    now_str,
                ),
            )
            .await?;
        }

        Ok(())
    }
}
