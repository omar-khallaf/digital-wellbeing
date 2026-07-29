//! Domain types for the categorization feature.

use futures::future::BoxFuture;
use wellbeing_core::{AppClass, Category};

/// Default category used when no category is assigned.
pub const DEFAULT_CATEGORY: Category = Category::Uncategorized;

/// A classifier that can categorize an app into a [`Category`].
pub trait AiClassifier: Send + Sync + 'static {
    fn classify(
        &self,
        app_class: AppClass,
        title: Option<String>,
    ) -> BoxFuture<'static, Option<Category>>;
}

/// Source of a category assignment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CategorySource {
    AppCategory {
        app_class: AppClass,
        category: Category,
    },
    AiClassified {
        app_class: AppClass,
        category: Category,
    },
    Uncategorized,
}

/// Heuristic keyword-based classifier.
#[derive(Debug, Clone)]
pub struct HeuristicClassifier;

impl HeuristicClassifier {
    pub(crate) fn match_keywords(app_class: &str) -> Option<Category> {
        let lower = app_class.to_lowercase();

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
            return Some(Category::Productivity);
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
            return Some(Category::Development);
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
            return Some(Category::Social);
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
            return Some(Category::Communication);
        }

        const ENTERTAINMENT_KW: &[&str] = &[
            "spotify", "steam", "youtube", "yt", "netflix", "vlc", "mpv", "twitch",
        ];
        if ENTERTAINMENT_KW.iter().any(|kw| lower.contains(kw)) {
            return Some(Category::Entertainment);
        }

        None
    }
}

impl AiClassifier for HeuristicClassifier {
    fn classify(
        &self,
        app_class: AppClass,
        _: Option<String>,
    ) -> BoxFuture<'static, Option<Category>> {
        let result = Self::match_keywords(app_class.as_str());
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
            let id = AppClass::new(app).unwrap();
            let got = HeuristicClassifier::match_keywords(id.as_str());
            assert_eq!(
                got,
                Some(Category::Productivity),
                "{app} should be Productivity"
            );
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
            let id = AppClass::new(app).unwrap();
            let got = HeuristicClassifier::match_keywords(id.as_str());
            assert_eq!(
                got,
                Some(Category::Development),
                "{app} should be Development"
            );
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
            let id = AppClass::new(app).unwrap();
            let got = HeuristicClassifier::match_keywords(id.as_str());
            assert_eq!(got, Some(Category::Social), "{app} should be Social");
        }
    }

    #[test]
    fn heuristic_communication() {
        let cases = &["slack", "discord", "telegram", "element", "signal"];
        for &app in cases {
            let id = AppClass::new(app).unwrap();
            let got = HeuristicClassifier::match_keywords(id.as_str());
            assert_eq!(
                got,
                Some(Category::Communication),
                "{app} should be Communication"
            );
        }
    }

    #[test]
    fn heuristic_entertainment() {
        let cases = &["spotify", "steam", "youtube", "vlc", "twitch"];
        for &app in cases {
            let id = AppClass::new(app).unwrap();
            let got = HeuristicClassifier::match_keywords(id.as_str());
            assert_eq!(
                got,
                Some(Category::Entertainment),
                "{app} should be Entertainment"
            );
        }
    }

    #[test]
    fn heuristic_unknown_returns_none() {
        let id = AppClass::new("unknown-app-12345").unwrap();
        assert_eq!(HeuristicClassifier::match_keywords(id.as_str()), None);
    }

    #[test]
    fn category_source_variants() {
        let app_class = AppClass::new("test").unwrap();
        let cat = Category::Productivity;

        let ac = CategorySource::AppCategory {
            app_class: app_class.clone(),
            category: cat,
        };
        let ai = CategorySource::AiClassified {
            app_class: app_class.clone(),
            category: cat,
        };
        let uncat = CategorySource::Uncategorized;

        assert_ne!(ac, ai);
        assert_ne!(ai, uncat);
        assert_ne!(ac, uncat);

        let ac2 = CategorySource::AppCategory {
            app_class: app_class.clone(),
            category: cat,
        };
        assert_eq!(ac, ac2);
    }
}
