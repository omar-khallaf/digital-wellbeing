//! Categorization engine — resolves app to category via DB, cache, or AI.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use diesel::JoinOnDsl;
use tokio::sync::Mutex;
use tracing::warn;
use wellbeing_core::{AppClass, CategoryId, Uid};

use crate::store::DbPool;

use super::domain::{AiClassifier, CategorySource};

const CACHE_TTL: Duration = Duration::from_secs(60);
const SYSTEM_USER_ID: i32 = 0;

type CacheMap = HashMap<AppClass, (CategoryId, Instant)>;

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
        if let Some(category_id) = self.ai.classify(owned_id, owned_title).await {
            if let Ok(mut cache) = self.cache.try_lock() {
                cache.insert(app_class.clone(), (category_id, Instant::now()));
            }
            return CategorySource::AiClassified {
                app_class: app_class.clone(),
                category_id,
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
        use diesel::ExpressionMethods;
        use diesel::QueryDsl;
        use diesel_async::RunQueryDsl;

        use crate::store::schema::{app_categories, apps};

        let mut conn = self.pool.get().await.ok()?;

        let result = app_categories::table
            .inner_join(apps::table.on(apps::id.eq(app_categories::app_id)))
            .filter(apps::app_class.eq(app_class.as_str()))
            .filter(app_categories::user_id.eq(uid.0 as i32))
            .select((app_categories::category_id, app_categories::ignore))
            .get_result::<(Option<i32>, bool)>(&mut conn)
            .await;

        match result {
            Ok(row) => {
                return match row {
                    (Some(cat_id), false) => Some(CategorySource::AppCategory {
                        app_class: app_class.clone(),
                        category_id: CategoryId(cat_id as i64),
                    }),
                    _ => Some(CategorySource::Uncategorized),
                };
            }
            Err(diesel::result::Error::NotFound) => {}
            Err(e) => {
                warn!(?e, %app_class, "categorization user-specific DB lookup failed");
            }
        }

        let result = app_categories::table
            .inner_join(apps::table.on(apps::id.eq(app_categories::app_id)))
            .filter(apps::app_class.eq(app_class.as_str()))
            .filter(app_categories::user_id.eq(SYSTEM_USER_ID))
            .select((app_categories::category_id, app_categories::ignore))
            .get_result::<(Option<i32>, bool)>(&mut conn)
            .await;

        match result {
            Ok((Some(cat_id), false)) => Some(CategorySource::AppCategory {
                app_class: app_class.clone(),
                category_id: CategoryId(cat_id as i64),
            }),
            Ok(_) => Some(CategorySource::Uncategorized),
            Err(diesel::result::Error::NotFound) => None,
            Err(e) => {
                warn!(?e, %app_class, "categorization DB fallback lookup failed");
                None
            }
        }
    }

    async fn lookup_cache(&self, app_class: &AppClass) -> Option<CategorySource> {
        let mut cache = self.cache.lock().await;
        if let Some(&(category_id, inserted_at)) = cache.get(app_class) {
            if inserted_at.elapsed() < CACHE_TTL {
                return Some(CategorySource::AiClassified {
                    app_class: app_class.clone(),
                    category_id,
                });
            }
            cache.remove(app_class);
        }
        None
    }
}
