//! Category query helpers for D-Bus interface.

use diesel::{ExpressionMethods, QueryDsl, insert_into, update};
use diesel_async::RunQueryDsl;
use wellbeing_core::{AppCategoryRow, Category, CategoryId};

use crate::store::DbPool;
use crate::store::schema::{app_categories, categories};

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
