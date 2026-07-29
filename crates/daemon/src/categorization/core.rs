//! Categorization engine — resolves app to category via DB, cache, or AI.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::Mutex;
use wellbeing_core::{AppClass, Category, Uid};

use crate::store::DbPool;
use crate::store::daos::categories::CategoryDao;

use super::domain::{AiClassifier, CategorySource, DEFAULT_CATEGORY};

const CACHE_TTL: Duration = Duration::from_secs(60);

type CacheMap = HashMap<AppClass, (Category, Instant)>;

/// Categorizer that resolves app categorization through multiple layers.
pub struct Categorizer<C: AiClassifier> {
    pool: DbPool,
    ai: Arc<C>,
    cache: Mutex<CacheMap>,
}

impl<C: AiClassifier> Categorizer<C> {
    pub fn new(pool: DbPool, ai: Arc<C>) -> Self {
        Self {
            pool,
            ai,
            cache: Mutex::new(HashMap::new()),
        }
    }

    pub async fn categorize(
        &self,
        app_class: &AppClass,
        title: Option<&str>,
        uid: Uid,
    ) -> CategorySource {
        if let Some(source) = self.lookup_db(app_class, uid).await {
            return source;
        }

        if let Some(source) = self.lookup_cache(app_class).await {
            return source;
        }

        let owned_id = app_class.clone();
        let owned_title = title.map(|s| s.to_string());
        if let Some(category) = self.ai.classify(owned_id, owned_title).await {
            if let Ok(mut cache) = self.cache.try_lock() {
                cache.insert(app_class.clone(), (category, Instant::now()));
            }
            return CategorySource::AiClassified {
                app_class: app_class.clone(),
                category,
            };
        }

        CategorySource::Uncategorized
    }

    pub fn invalidate(&self, app_class: &AppClass) {
        if let Ok(mut cache) = self.cache.try_lock() {
            cache.remove(app_class);
        }
    }

    async fn lookup_db(&self, app_class: &AppClass, uid: Uid) -> Option<CategorySource> {
        let conn = self.pool.client().await.ok()?;

        // Try user-specific categories first
        let result = CategoryDao::resolve_categories_for_app(&conn, app_class, uid).await;

        match result {
            Ok(categories) if !categories.is_empty() => {
                return Some(CategorySource::AppCategory {
                    app_class: app_class.clone(),
                    category: categories.into_iter().next().unwrap_or(DEFAULT_CATEGORY),
                });
            }
            _ => {}
        }

        // Try system-global fallback (user_id = 0)
        let result = CategoryDao::resolve_categories_for_app(&conn, app_class, Uid(0)).await;

        match result {
            Ok(categories) if !categories.is_empty() => {
                return Some(CategorySource::AppCategory {
                    app_class: app_class.clone(),
                    category: categories.into_iter().next().unwrap_or(DEFAULT_CATEGORY),
                });
            }
            _ => {}
        }

        None
    }

    async fn lookup_cache(&self, app_class: &AppClass) -> Option<CategorySource> {
        let mut cache = self.cache.lock().await;
        if let Some((category, inserted_at)) = cache.get(app_class) {
            if inserted_at.elapsed() < CACHE_TTL {
                return Some(CategorySource::AiClassified {
                    app_class: app_class.clone(),
                    category: *category,
                });
            }
            cache.remove(app_class);
        }
        None
    }
}
