use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::future::BoxFuture;
use tokio::sync::Mutex;
use tracing::warn;
use wellbeing_core::{AppId, CategoryId, Uid};

use crate::store::DbPool;

const CACHE_TTL: Duration = Duration::from_secs(60);

type CacheMap = HashMap<AppId, (CategoryId, Instant)>;

pub trait AiClassifier: Send + Sync + 'static {
    fn classify(
        &self,
        app_id: AppId,
        title: Option<String>,
    ) -> BoxFuture<'static, Option<CategoryId>>;
}

#[derive(Debug, Clone)]
pub struct HeuristicClassifier;

impl HeuristicClassifier {
    const PRODUCTIVITY: i64 = 1;
    const COMMUNICATION: i64 = 2;
    const ENTERTAINMENT: i64 = 3;
    const SOCIAL: i64 = 4;
    const DEVELOPMENT: i64 = 5;

    pub(crate) fn match_keywords(app_id: &str) -> Option<CategoryId> {
        let lower = app_id.to_lowercase();

        const PRODUCTIVITY_KW: &[&str] = &[
            "alacritty",
            "kitty",
            "foot",
            "wezterm",
            "gnome-terminal",
            "konsole",
            "terminator",
            "tmux",
        ];
        if PRODUCTIVITY_KW.iter().any(|kw| lower.contains(kw)) {
            return Some(CategoryId(Self::PRODUCTIVITY));
        }

        const DEVELOPMENT_KW: &[&str] = &[
            "code",
            "idea",
            "nvim",
            "neovim",
            "emacs",
            "sublime",
            "atom",
            "zed",
            "jetbrains",
            "android-studio",
            "vim",
            "helix",
        ];
        if DEVELOPMENT_KW.iter().any(|kw| lower.contains(kw)) {
            return Some(CategoryId(Self::DEVELOPMENT));
        }

        const SOCIAL_KW: &[&str] = &[
            "firefox",
            "chrome",
            "chromium",
            "brave",
            "zen-browser",
            "edge",
            "opera",
        ];
        if SOCIAL_KW.iter().any(|kw| lower.contains(kw)) {
            return Some(CategoryId(Self::SOCIAL));
        }

        const COMMUNICATION_KW: &[&str] = &[
            "slack",
            "discord",
            "telegram",
            "element",
            "signal",
            "whatsapp",
            "messenger",
            "thunderbird",
            "outlook",
        ];
        if COMMUNICATION_KW.iter().any(|kw| lower.contains(kw)) {
            return Some(CategoryId(Self::COMMUNICATION));
        }

        const ENTERTAINMENT_KW: &[&str] = &[
            "spotify", "steam", "youtube", "yt", "netflix", "vlc", "mpv", "twitch",
        ];
        if ENTERTAINMENT_KW.iter().any(|kw| lower.contains(kw)) {
            return Some(CategoryId(Self::ENTERTAINMENT));
        }

        None
    }
}

impl AiClassifier for HeuristicClassifier {
    fn classify(
        &self,
        app_id: AppId,
        _title: Option<String>,
    ) -> BoxFuture<'static, Option<CategoryId>> {
        let result = Self::match_keywords(app_id.as_str());
        Box::pin(async move { result })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CategorySource {
    AppCategory {
        app_id: AppId,
        category_id: CategoryId,
    },
    AiClassified {
        app_id: AppId,
        category_id: CategoryId,
    },
    Uncategorized,
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use wellbeing_core::AppId;

    #[test]
    fn heuristic_productivity() {
        let cases = &[
            "Alacritty",
            "kitty",
            "foot",
            "wezterm",
            "gnome-terminal",
            "tmux",
        ];
        for &app in cases {
            let id = AppId::new(app).unwrap();
            let got = HeuristicClassifier::match_keywords(id.as_str());
            assert_eq!(got, Some(CategoryId(1)), "{app} should be Productivity");
        }
    }

    #[test]
    fn heuristic_development() {
        let cases = &[
            "Code",
            "code-oss",
            "jetbrains-idea",
            "nvim",
            "emacs",
            "zed",
            "helix",
        ];
        for &app in cases {
            let id = AppId::new(app).unwrap();
            let got = HeuristicClassifier::match_keywords(id.as_str());
            assert_eq!(got, Some(CategoryId(5)), "{app} should be Development");
        }
    }

    #[test]
    fn heuristic_social() {
        let cases = &[
            "firefox",
            "Google-chrome",
            "chromium-browser",
            "brave-browser",
            "zen-browser",
        ];
        for &app in cases {
            let id = AppId::new(app).unwrap();
            let got = HeuristicClassifier::match_keywords(id.as_str());
            assert_eq!(got, Some(CategoryId(4)), "{app} should be Social");
        }
    }

    #[test]
    fn heuristic_communication() {
        let cases = &["slack", "discord", "telegram", "element", "signal"];
        for &app in cases {
            let id = AppId::new(app).unwrap();
            let got = HeuristicClassifier::match_keywords(id.as_str());
            assert_eq!(got, Some(CategoryId(2)), "{app} should be Communication");
        }
    }

    #[test]
    fn heuristic_entertainment() {
        let cases = &["spotify", "steam", "youtube", "vlc", "twitch"];
        for &app in cases {
            let id = AppId::new(app).unwrap();
            let got = HeuristicClassifier::match_keywords(id.as_str());
            assert_eq!(got, Some(CategoryId(3)), "{app} should be Entertainment");
        }
    }

    #[test]
    fn heuristic_unknown_returns_none() {
        let id = AppId::new("unknown-app-12345").unwrap();
        assert_eq!(HeuristicClassifier::match_keywords(id.as_str()), None);
    }

    #[test]
    fn category_source_variants() {
        let app_id = AppId::new("test").unwrap();
        let cat_id = CategoryId(42);

        let ac = CategorySource::AppCategory {
            app_id: app_id.clone(),
            category_id: cat_id,
        };
        let ai = CategorySource::AiClassified {
            app_id: app_id.clone(),
            category_id: cat_id,
        };
        let uncat = CategorySource::Uncategorized;

        assert_ne!(ac, ai);
        assert_ne!(ai, uncat);
        assert_ne!(ac, uncat);

        let ac2 = CategorySource::AppCategory {
            app_id: app_id.clone(),
            category_id: cat_id,
        };
        assert_eq!(ac, ac2);
    }
}
