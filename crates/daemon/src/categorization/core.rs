//! Categorization engine — resolves app to category via DB, cache, or AI.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::Mutex;
use tracing::warn;
use wellbeing_core::{AppId, CategoryId, Uid};

use crate::store::DbPool;

use super::domain::{AiClassifier, CategorySource};

const CACHE_TTL: Duration = Duration::from_secs(60);

type CacheMap = HashMap<AppId, (CategoryId, Instant)>;

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
        app_id: &AppId,
        title: Option<&str>,
        uid: Uid,
    ) -> CategorySource {
        if let Some(source) = self.lookup_db(app_id, uid).await {
            return source;
        }

        if let Some(source) = self.lookup_cache(app_id).await {
            return source;
        }

        let owned_id = app_id.clone();
        let owned_title = title.map(|s| s.to_string());
        if let Some(category_id) = self.ai.classify(owned_id, owned_title).await {
            if let Ok(mut cache) = self.cache.try_lock() {
                cache.insert(app_id.clone(), (category_id, Instant::now()));
            }
            return CategorySource::AiClassified {
                app_id: app_id.clone(),
                category_id,
            };
        }

        CategorySource::Uncategorized
    }

    pub fn invalidate(&self, app_id: &AppId) {
        if let Ok(mut cache) = self.cache.try_lock() {
            cache.remove(app_id);
        }
    }

    async fn lookup_db(&self, app_id: &AppId, uid: Uid) -> Option<CategorySource> {
        use diesel::ExpressionMethods;
        use diesel::QueryDsl;
        use diesel_async::RunQueryDsl;

        use crate::store::schema::app_categories;

        let mut conn = self.pool.get().await.ok()?;

        let result = app_categories::table
            .filter(app_categories::app_id.eq(app_id.as_str()))
            .filter(app_categories::user_id.eq(uid.0 as i32))
            .select((app_categories::category_id, app_categories::ignore))
            .get_result::<(Option<i32>, bool)>(&mut conn)
            .await;

        match result {
            Ok(row) => {
                return match row {
                    (Some(cat_id), false) => Some(CategorySource::AppCategory {
                        app_id: app_id.clone(),
                        category_id: CategoryId(cat_id as i64),
                    }),
                    _ => Some(CategorySource::Uncategorized),
                };
            }
            Err(diesel::result::Error::NotFound) => {}
            Err(e) => {
                warn!(?e, %app_id, "categorization user-specific DB lookup failed");
            }
        }

        let result = app_categories::table
            .filter(app_categories::app_id.eq(app_id.as_str()))
            .filter(app_categories::user_id.eq(0i32))
            .select((app_categories::category_id, app_categories::ignore))
            .get_result::<(Option<i32>, bool)>(&mut conn)
            .await;

        match result {
            Ok((Some(cat_id), false)) => Some(CategorySource::AppCategory {
                app_id: app_id.clone(),
                category_id: CategoryId(cat_id as i64),
            }),
            Ok(_) => Some(CategorySource::Uncategorized),
            Err(diesel::result::Error::NotFound) => None,
            Err(e) => {
                warn!(?e, %app_id, "categorization DB fallback lookup failed");
                None
            }
        }
    }

    async fn lookup_cache(&self, app_id: &AppId) -> Option<CategorySource> {
        let mut cache = self.cache.lock().await;
        if let Some(&(category_id, inserted_at)) = cache.get(app_id) {
            if inserted_at.elapsed() < CACHE_TTL {
                return Some(CategorySource::AiClassified {
                    app_id: app_id.clone(),
                    category_id,
                });
            }
            cache.remove(app_id);
        }
        None
    }
}
