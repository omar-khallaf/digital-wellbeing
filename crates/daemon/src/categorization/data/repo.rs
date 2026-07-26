//! Category repository — wraps store DAO calls with domain-type conversion.
//!
//! [`CategoryDao`] already returns domain types (`wellbeing_core::Category`,
//! `wellbeing_core::AppCategoryRow`), so this repo is a thin pass-through
//! that provides a consistent repository interface for the feature.

use wellbeing_core::{AppCategoryRow, AppClass, Category, CategoryId, Uid};

use crate::store::DbPool;
use crate::store::daos::categories::CategoryDao;

/// Repository for category CRUD and lookups.
pub struct CategorizationRepo {
    pool: DbPool,
}

impl CategorizationRepo {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    /// List all system-defined categories.
    pub async fn list_categories(&self) -> anyhow::Result<Vec<Category>> {
        let mut conn = self.pool.get().await?;
        CategoryDao::list_all_categories(&mut conn).await
    }

    /// Get app-category overrides for a user.
    pub async fn list_app_categories(&self, caller: Uid) -> anyhow::Result<Vec<AppCategoryRow>> {
        let mut conn = self.pool.get().await?;
        CategoryDao::list_app_categories(&mut conn, caller.0).await
    }

    /// Set or create an app-category override.
    pub async fn set_app_category(
        &self,
        app_class: &AppClass,
        category_id: CategoryId,
        caller: Uid,
        now_str: &str,
    ) -> anyhow::Result<()> {
        let mut conn = self.pool.get().await?;
        CategoryDao::upsert_app_category(
            &mut conn,
            app_class.to_string(),
            category_id,
            caller.0,
            now_str,
        )
        .await
    }
}
