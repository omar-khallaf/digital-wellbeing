//! Categories repository — the `categories` and `app_categories` tables.
//!
//! The `categories` table is system-defined and read-only for users.
//! Only `app_categories` (user-specific category overrides) is mutable.

use diesel::ExpressionMethods;
use diesel::JoinOnDsl;
use diesel::QueryDsl;
use diesel::dsl::{insert_into, update};
use diesel_async::RunQueryDsl;
use wellbeing_core::Uid;
use wellbeing_core::{AppCategoryRow, AppClass, Category, CategoryId};

use crate::store::connection::DbConn;
use crate::store::schema;

/// System-level user id sentinel (root / no specific user).
const SYSTEM_USER_ID: i32 = 0;
/// Fallback category id if "Uncategorized" query fails.
const UNCATEGORIZED_FALLBACK_ID: i32 = 7;

/// Diesel-backed category repository — provides static methods.
pub(crate) struct CategoryDao;

// ── Internal / transaction-friendly methods ─────────────────────────────
impl CategoryDao {
    /// Resolve category ids for a batch of (uid, app_id) pairs using an
    /// existing connection. User-specific entries take precedence; system-global
    /// (user_id = 0) entries are fallback. Unmapped apps get "Uncategorized".
    pub(crate) async fn resolve_category_map(
        conn: &mut DbConn,
        pairs: &[(Uid, i32)],
    ) -> std::collections::HashMap<(Uid, i32), Vec<i32>> {
        use diesel::QueryDsl;
        let mut map: std::collections::HashMap<(Uid, i32), Vec<i32>> =
            std::collections::HashMap::new();

        // Cache the Uncategorized category id — look it up once.
        let uncategorized_id: i32 = schema::categories::table
            .filter(schema::categories::name.eq("Uncategorized"))
            .select(schema::categories::id)
            .first(conn)
            .await
            .unwrap_or(UNCATEGORIZED_FALLBACK_ID); // fallback id if query fails

        for &(uid, app_id) in pairs {
            let user_cats: Vec<i32> = schema::app_categories::table
                .filter(schema::app_categories::app_id.eq(app_id))
                .filter(schema::app_categories::user_id.eq(uid.0 as i32))
                .filter(schema::app_categories::ignore.eq(false))
                .filter(schema::app_categories::category_id.is_not_null())
                .select(schema::app_categories::category_id)
                .load::<Option<i32>>(conn)
                .await
                .unwrap_or_default()
                .into_iter()
                .flatten()
                .collect();

            let ids = if !user_cats.is_empty() {
                user_cats
            } else {
                let sys_cats: Vec<i32> = schema::app_categories::table
                    .filter(schema::app_categories::app_id.eq(app_id))
                    .filter(schema::app_categories::user_id.eq(SYSTEM_USER_ID))
                    .filter(schema::app_categories::ignore.eq(false))
                    .filter(schema::app_categories::category_id.is_not_null())
                    .select(schema::app_categories::category_id)
                    .load::<Option<i32>>(conn)
                    .await
                    .unwrap_or_default()
                    .into_iter()
                    .flatten()
                    .collect();
                if sys_cats.is_empty() {
                    vec![uncategorized_id]
                } else {
                    sys_cats
                }
            };
            map.insert((uid, app_id), ids);
        }

        map
    }

    /// Resolve category names for an app (user-specific then system fallback).
    pub(crate) async fn resolve_category_names_for_app(
        conn: &mut DbConn,
        app_class: &wellbeing_core::AppClass,
        uid: Uid,
    ) -> anyhow::Result<Vec<String>> {
        use diesel::ExpressionMethods;
        use diesel::QueryDsl;
        use diesel_async::RunQueryDsl;

        let cat_ids: Vec<i32> = schema::app_categories::table
            .inner_join(schema::apps::table)
            .filter(schema::apps::app_class.eq(app_class.as_ref()))
            .filter(schema::app_categories::category_id.is_not_null())
            .filter(schema::app_categories::ignore.eq(false))
            .filter(schema::app_categories::user_id.eq(uid.0 as i32))
            .select(schema::app_categories::category_id)
            .load::<Option<i32>>(conn)
            .await?
            .into_iter()
            .flatten()
            .collect();

        if !cat_ids.is_empty() {
            let names: Vec<String> = schema::categories::table
                .filter(schema::categories::id.eq_any(&cat_ids))
                .select(schema::categories::name)
                .load(conn)
                .await?;
            return Ok(names);
        }

        // System-global fallback (user_id = 0).
        let fallback_ids: Vec<i32> = schema::app_categories::table
            .inner_join(schema::apps::table)
            .filter(schema::apps::app_class.eq(app_class.as_ref()))
            .filter(schema::app_categories::category_id.is_not_null())
            .filter(schema::app_categories::ignore.eq(false))
            .filter(schema::app_categories::user_id.eq(0i32))
            .select(schema::app_categories::category_id)
            .load::<Option<i32>>(conn)
            .await?
            .into_iter()
            .flatten()
            .collect();

        if !fallback_ids.is_empty() {
            let names: Vec<String> = schema::categories::table
                .filter(schema::categories::id.eq_any(&fallback_ids))
                .select(schema::categories::name)
                .load(conn)
                .await?;
            Ok(names)
        } else {
            Ok(Vec::new())
        }
    }
}

// ── DAO functions (replace raw SQL in dbus layer) ───────────────────────

impl CategoryDao {
    /// List all system-defined categories.
    pub(crate) async fn list_all_categories(conn: &mut DbConn) -> anyhow::Result<Vec<Category>> {
        use crate::store::schema::categories::columns as c;
        let rows: Vec<(i32, String, Option<String>, Option<String>)> =
            crate::store::schema::categories::table
                .select((c::id, c::name, c::color, c::icon))
                .load(conn)
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

    /// Get app_categories for a user (user-specific overrides).
    pub(crate) async fn list_app_categories(
        conn: &mut DbConn,
        caller: u32,
    ) -> anyhow::Result<Vec<AppCategoryRow>> {
        use crate::store::schema::app_categories;
        use crate::store::schema::apps;

        type AppCategoryRowRaw = (
            String,         // apps.app_class (window class)
            i32,            // user_id
            Option<i32>,    // category_id
            Option<String>, // display_name
            Option<String>, // icon_path
            bool,           // ignore
        );
        let rows: Vec<AppCategoryRowRaw> = app_categories::table
            .inner_join(apps::table.on(apps::id.eq(app_categories::app_id)))
            .filter(app_categories::user_id.eq(caller as i32))
            .select((
                apps::app_class,
                app_categories::user_id,
                app_categories::category_id,
                app_categories::display_name,
                app_categories::icon_path,
                app_categories::ignore,
            ))
            .load(conn)
            .await?;

        Ok(rows
            .into_iter()
            .map(
                |(app_class, uid, cat_id, display_name, icon_path, ignore)| AppCategoryRow {
                    app_class: AppClass::new(&app_class)
                        .unwrap_or_else(|_| AppClass::new("?").unwrap()),
                    user_id: Uid(uid as u32),
                    category_id: CategoryId(cat_id.unwrap_or(0) as i64),
                    display_name: display_name.unwrap_or_default(),
                    icon_path: icon_path.unwrap_or_default(),
                    ignore,
                },
            )
            .collect())
    }

    /// Set or create an app category override (upsert).
    pub(crate) async fn upsert_app_category(
        conn: &mut DbConn,
        app_class: String,
        category_id: CategoryId,
        caller: u32,
        now_str: &str,
    ) -> anyhow::Result<()> {
        use crate::store::schema::app_categories;
        use crate::store::schema::apps;

        let uid = caller as i32;
        let cat_id: Option<i32> = if category_id.0 > 0 {
            Some(category_id.0 as i32)
        } else {
            None
        };

        // Resolve app_id via subquery against the window-class string.
        let app_id: i32 = apps::table
            .filter(apps::app_class.eq(&app_class))
            .select(apps::id)
            .first(conn)
            .await?;

        let updated = update(
            app_categories::table
                .filter(app_categories::app_id.eq(app_id))
                .filter(app_categories::user_id.eq(uid)),
        )
        .set((
            app_categories::category_id.eq(cat_id),
            app_categories::updated_at.eq(now_str),
        ))
        .execute(conn)
        .await?;

        if updated == 0 {
            insert_into(app_categories::table)
                .values((
                    app_categories::app_id.eq(app_id),
                    app_categories::user_id.eq(uid),
                    app_categories::category_id.eq(cat_id),
                    app_categories::display_name.eq(None::<String>),
                    app_categories::icon_path.eq(None::<String>),
                    app_categories::ignore.eq(false),
                    app_categories::updated_at.eq(now_str),
                ))
                .execute(conn)
                .await?;
        }

        Ok(())
    }
}
