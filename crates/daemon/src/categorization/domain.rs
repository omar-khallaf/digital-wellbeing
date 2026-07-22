//! Domain types for the categorization feature.

use futures::future::BoxFuture;
use wellbeing_core::{AppId, CategoryId};

/// A classifier that can categorize an app into a category.
pub trait AiClassifier: Send + Sync + 'static {
    fn classify(
        &self,
        app_id: AppId,
        title: Option<String>,
    ) -> BoxFuture<'static, Option<CategoryId>>;
}

/// Source of a category assignment.
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

/// Heuristic keyword-based classifier.
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
    fn classify(&self, app_id: AppId, _: Option<String>) -> BoxFuture<'static, Option<CategoryId>> {
        let result = Self::match_keywords(app_id.as_str());
        Box::pin(async move { result })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
