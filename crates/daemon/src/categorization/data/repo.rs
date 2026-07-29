//! Category repository — wraps store DAO calls with domain-type conversion.
//!
//! [`CategoryDao`] returns domain types (`wellbeing_core::Category` enum,
//! `wellbeing_core::AppCategoryRow`), so this repo is a thin pass-through
//! that provides a consistent repository interface for the feature.

use wellbeing_core::{AppCategoryRow, AppClass, Category, Uid};

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

    /// List all known categories.
    pub async fn list_categories(&self) -> anyhow::Result<Vec<Category>> {
        Ok(Category::ALL.to_vec())
    }

    /// Get app-category overrides for a user.
    pub async fn list_app_categories(&self, caller: Uid) -> anyhow::Result<Vec<AppCategoryRow>> {
        let conn = self.pool.client().await?;
        CategoryDao::list_app_categories(&conn, caller.0).await
    }

    /// Set or create an app-category override.
    pub async fn set_app_category(
        &self,
        app_class: &AppClass,
        category: Category,
        caller: Uid,
        now_str: &str,
    ) -> anyhow::Result<()> {
        let conn = self.pool.client().await?;
        CategoryDao::upsert_app_category(&conn, app_class.to_string(), category, caller.0, now_str)
            .await
    }
}
