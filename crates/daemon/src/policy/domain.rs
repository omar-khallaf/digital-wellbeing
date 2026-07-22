//! Domain types for the policy feature.
//!
//! These types represent the core domain model: time-limited apps,
//! tracked apps, policy configurations, time windows, and verdicts.

use chrono::{DateTime, Utc};
pub use wellbeing_core::{AppId, BlockReason, CategoryId, PolicyId};

/// App with configured time limit and optional extension.
pub enum TimeLimitedApp {
    Normal(i64, i64),
    Extended(i64, i64),
}

impl TimeLimitedApp {
    pub fn remaining(&self) -> i64 {
        match self {
            Self::Normal(used, limit) | Self::Extended(used, limit) => limit - used,
        }
    }

    pub fn can_extend(&self) -> bool {
        matches!(self, Self::Normal(..))
    }

    pub fn effective_limit(&self) -> i64 {
        match self {
            Self::Normal(_, limit) | Self::Extended(_, limit) => *limit,
        }
    }
}

/// App with tracked (non-blocking) time usage.
pub struct TimeTrackedApp {
    pub used: i64,
    pub limit: i64,
}

impl TimeTrackedApp {
    pub fn remaining(&self) -> i64 {
        (self.limit - self.used).max(0)
    }

    pub fn is_exceeded(&self) -> bool {
        self.used >= self.limit
    }
}

/// Union of time-limited and time-tracked app states.
pub enum TrackedApp {
    TimeLimited(TimeLimitedApp),
    TimeTracked(TimeTrackedApp),
}

/// Result of evaluating policies for an app.
#[must_use]
pub enum PolicyVerdict {
    Ok,
    Block {
        policy_id: PolicyId,
        reason: BlockReason,
        remaining: i64,
    },
    Notify {
        policy_id: PolicyId,
        repeat_interval: Option<i64>,
    },
}

/// Configuration for a policy, desugared from the database row.
#[derive(Debug, Clone)]
pub enum PolicyConfig {
    Block {
        id: PolicyId,
        app_id: Option<AppId>,
        category_id: Option<CategoryId>,
        active: bool,
    },
    TimeLimit {
        id: PolicyId,
        app_id: Option<AppId>,
        category_id: Option<CategoryId>,
        time_limit_minutes: i64,
        extra_minutes: i64,
        active: bool,
    },
    Notify {
        id: PolicyId,
        app_id: Option<AppId>,
        category_id: Option<CategoryId>,
        time_limit_minutes: i64,
        notification_repeat_interval_minutes: Option<i64>,
        active: bool,
    },
}

impl PolicyConfig {
    pub fn id(&self) -> PolicyId {
        match self {
            Self::Block { id, .. } | Self::TimeLimit { id, .. } | Self::Notify { id, .. } => *id,
        }
    }

    pub fn app_id(&self) -> Option<&AppId> {
        match self {
            Self::Block { app_id, .. }
            | Self::TimeLimit { app_id, .. }
            | Self::Notify { app_id, .. } => app_id.as_ref(),
        }
    }

    pub fn category_id(&self) -> Option<CategoryId> {
        match self {
            Self::Block { category_id, .. }
            | Self::TimeLimit { category_id, .. }
            | Self::Notify { category_id, .. } => *category_id,
        }
    }

    pub fn active(&self) -> bool {
        match self {
            Self::Block { active, .. }
            | Self::TimeLimit { active, .. }
            | Self::Notify { active, .. } => *active,
        }
    }
}

// ═════════════════════════════════════════════════════════════════════════════
// Domain types belonging to the policy feature.
// ═════════════════════════════════════════════════════════════════════════════

use wellbeing_core::TimeWindow;

/// Shared metadata attached to every policy variant.
#[derive(Debug, Clone)]
pub struct PolicyMeta {
    pub id: PolicyId,
    pub name: String,
    pub time_windows: Option<TimeWindow>,
    pub active: bool,
    pub created_by: u32,
    pub owner_id: u32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Target for an app-scoped policy.
#[derive(Debug, Clone)]
pub struct AppTarget {
    pub app_id: AppId,
}

/// Target for a category-scoped policy.
#[derive(Debug, Clone)]
pub struct CategoryTarget {
    pub category_id: CategoryId,
}

/// Action taken by an app-scoped policy.
#[derive(Debug, Clone)]
pub enum AppAction {
    Block,
    TimeLimit {
        limit_minutes: i64,
        extra_minutes: i64,
    },
    Notify {
        limit_minutes: i64,
        repeat_interval_minutes: Option<i64>,
    },
}

/// Action taken by a category-scoped policy.
#[derive(Debug, Clone)]
pub enum CategoryAction {
    Block,
    TimeLimit {
        limit_minutes: i64,
        extra_minutes: i64,
    },
    Notify {
        limit_minutes: i64,
        repeat_interval_minutes: Option<i64>,
    },
}

/// An app-scoped policy: targets a specific application.
#[derive(Debug, Clone)]
pub struct AppPolicy {
    pub target: AppTarget,
    pub meta: PolicyMeta,
    pub action: AppAction,
}

/// A category-scoped policy: targets every app in a category.
#[derive(Debug, Clone)]
pub struct CategoryPolicy {
    pub target: CategoryTarget,
    pub meta: PolicyMeta,
    pub action: CategoryAction,
}

/// Top-level domain policy — hierarchical design matching
/// `Policy::App(AppPolicy { action: AppAction::Block })`.
#[derive(Debug, Clone)]
pub enum Policy {
    App(Box<AppPolicy>),
    Category(Box<CategoryPolicy>),
}

impl Policy {
    pub fn id(&self) -> PolicyId {
        self.meta().id
    }

    pub fn meta(&self) -> &PolicyMeta {
        match self {
            Policy::App(p) => &p.meta,
            Policy::Category(p) => &p.meta,
        }
    }

    pub fn is_active(&self) -> bool {
        self.meta().active
    }

    pub fn time_windows(&self) -> Option<&TimeWindow> {
        self.meta().time_windows.as_ref()
    }

    /// Resolve app_id string for display / matching (empty = category policy).
    pub fn app_id_str(&self) -> String {
        match self {
            Policy::App(p) => p.target.app_id.as_ref().to_string(),
            Policy::Category(_) => String::new(),
        }
    }

    /// Resolve category_id (0 = app policy).
    pub fn category_id_val(&self) -> i64 {
        match self {
            Policy::App(_) => 0,
            Policy::Category(p) => p.target.category_id.0,
        }
    }

    /// Resolve limit in minutes (0 = Block variant).
    pub fn limit_minutes(&self) -> i64 {
        match self {
            Policy::App(p) => match p.action {
                AppAction::Block => 0,
                AppAction::TimeLimit { limit_minutes, .. } => limit_minutes,
                AppAction::Notify { limit_minutes, .. } => limit_minutes,
            },
            Policy::Category(p) => match p.action {
                CategoryAction::Block => 0,
                CategoryAction::TimeLimit { limit_minutes, .. } => limit_minutes,
                CategoryAction::Notify { limit_minutes, .. } => limit_minutes,
            },
        }
    }

    /// Resolve extra minutes (0 for Block / Notify).
    pub fn extra_minutes(&self) -> i64 {
        match self {
            Policy::App(p) => match p.action {
                AppAction::Block | AppAction::Notify { .. } => 0,
                AppAction::TimeLimit { extra_minutes, .. } => extra_minutes,
            },
            Policy::Category(p) => match p.action {
                CategoryAction::Block | CategoryAction::Notify { .. } => 0,
                CategoryAction::TimeLimit { extra_minutes, .. } => extra_minutes,
            },
        }
    }

    /// Resolve notification repeat interval (None for Block / TimeLimit).
    pub fn repeat_interval_minutes(&self) -> Option<i64> {
        match self {
            Policy::App(p) => match p.action {
                AppAction::Block | AppAction::TimeLimit { .. } => None,
                AppAction::Notify {
                    repeat_interval_minutes,
                    ..
                } => repeat_interval_minutes,
            },
            Policy::Category(p) => match p.action {
                CategoryAction::Block | CategoryAction::TimeLimit { .. } => None,
                CategoryAction::Notify {
                    repeat_interval_minutes,
                    ..
                } => repeat_interval_minutes,
            },
        }
    }
}

// ═════════════════════════════════════════════════════════════════════════════
// Conversions between D-Bus flat types and domain enums
// ═════════════════════════════════════════════════════════════════════════════

impl From<wellbeing_core::PolicyData> for Policy {
    fn from(p: wellbeing_core::PolicyData) -> Self {
        let meta = PolicyMeta {
            id: p.id,
            name: p.name,
            time_windows: if p.schedule_json.is_empty() {
                None
            } else {
                serde_json::from_str(&p.schedule_json).ok().flatten()
            },
            active: p.active,
            created_by: p.created_by,
            owner_id: p.owner_id,
            created_at: p.created_at.parse().ok().unwrap_or_else(Utc::now),
            updated_at: p.updated_at.parse().ok().unwrap_or_else(Utc::now),
        };

        let has_app = !p.app_id.is_empty();
        match (p.action, has_app) {
            (wellbeing_core::PolicyKind::Block, true) => Policy::App(Box::new(AppPolicy {
                target: AppTarget {
                    app_id: AppId::new(&p.app_id)
                        .unwrap_or_else(|_| AppId::new("unknown").unwrap()),
                },
                meta,
                action: AppAction::Block,
            })),
            (wellbeing_core::PolicyKind::Block, false) => {
                Policy::Category(Box::new(CategoryPolicy {
                    target: CategoryTarget {
                        category_id: CategoryId(p.category_id),
                    },
                    meta,
                    action: CategoryAction::Block,
                }))
            }
            (wellbeing_core::PolicyKind::TimeLimit, true) => Policy::App(Box::new(AppPolicy {
                target: AppTarget {
                    app_id: AppId::new(&p.app_id)
                        .unwrap_or_else(|_| AppId::new("unknown").unwrap()),
                },
                meta,
                action: AppAction::TimeLimit {
                    limit_minutes: p.time_limit_minutes.max(1),
                    extra_minutes: p.extra_minutes,
                },
            })),
            (wellbeing_core::PolicyKind::TimeLimit, false) => {
                Policy::Category(Box::new(CategoryPolicy {
                    target: CategoryTarget {
                        category_id: CategoryId(p.category_id),
                    },
                    meta,
                    action: CategoryAction::TimeLimit {
                        limit_minutes: p.time_limit_minutes.max(1),
                        extra_minutes: p.extra_minutes,
                    },
                }))
            }
            (wellbeing_core::PolicyKind::Notify, true) => Policy::App(Box::new(AppPolicy {
                target: AppTarget {
                    app_id: AppId::new(&p.app_id)
                        .unwrap_or_else(|_| AppId::new("unknown").unwrap()),
                },
                meta,
                action: AppAction::Notify {
                    limit_minutes: p.time_limit_minutes.max(1),
                    repeat_interval_minutes: if p.notification_repeat_interval_minutes == 0 {
                        None
                    } else {
                        Some(p.notification_repeat_interval_minutes)
                    },
                },
            })),
            (wellbeing_core::PolicyKind::Notify, false) => {
                Policy::Category(Box::new(CategoryPolicy {
                    target: CategoryTarget {
                        category_id: CategoryId(p.category_id),
                    },
                    meta,
                    action: CategoryAction::Notify {
                        limit_minutes: p.time_limit_minutes.max(1),
                        repeat_interval_minutes: if p.notification_repeat_interval_minutes == 0 {
                            None
                        } else {
                            Some(p.notification_repeat_interval_minutes)
                        },
                    },
                }))
            }
        }
    }
}

impl From<Policy> for wellbeing_core::PolicyData {
    fn from(p: Policy) -> Self {
        wellbeing_core::PolicyData {
            id: p.id(),
            name: p.meta().name.clone(),
            action: match &p {
                Policy::App(a) => match a.action {
                    AppAction::Block => wellbeing_core::PolicyKind::Block,
                    AppAction::TimeLimit { .. } => wellbeing_core::PolicyKind::TimeLimit,
                    AppAction::Notify { .. } => wellbeing_core::PolicyKind::Notify,
                },
                Policy::Category(c) => match c.action {
                    CategoryAction::Block => wellbeing_core::PolicyKind::Block,
                    CategoryAction::TimeLimit { .. } => wellbeing_core::PolicyKind::TimeLimit,
                    CategoryAction::Notify { .. } => wellbeing_core::PolicyKind::Notify,
                },
            },
            app_id: p.app_id_str(),
            category_id: p.category_id_val(),
            time_limit_minutes: p.limit_minutes(),
            extra_minutes: p.extra_minutes(),
            notification_repeat_interval_minutes: p.repeat_interval_minutes().unwrap_or(0),
            schedule_json: p
                .meta()
                .time_windows
                .as_ref()
                .and_then(|tw| serde_json::to_string(tw).ok())
                .unwrap_or_default(),
            active: p.is_active(),
            created_by: p.meta().created_by,
            owner_id: p.meta().owner_id,
            created_at: p.meta().created_at.to_rfc3339(),
            updated_at: p.meta().updated_at.to_rfc3339(),
        }
    }
}

impl From<Policy> for PolicyConfig {
    fn from(p: Policy) -> Self {
        let id = p.id();
        let active = p.is_active();

        let (app_id, category_id) = match &p {
            Policy::App(a) => (Some(a.target.app_id.clone()), None),
            Policy::Category(c) => (None, Some(c.target.category_id)),
        };

        match p {
            Policy::App(a) => match a.action {
                AppAction::Block => PolicyConfig::Block {
                    id,
                    app_id,
                    category_id,
                    active,
                },
                AppAction::TimeLimit {
                    limit_minutes,
                    extra_minutes,
                } => PolicyConfig::TimeLimit {
                    id,
                    app_id,
                    category_id,
                    time_limit_minutes: limit_minutes.max(1),
                    extra_minutes,
                    active,
                },
                AppAction::Notify {
                    limit_minutes,
                    repeat_interval_minutes,
                } => PolicyConfig::Notify {
                    id,
                    app_id,
                    category_id,
                    time_limit_minutes: limit_minutes.max(1),
                    notification_repeat_interval_minutes: repeat_interval_minutes,
                    active,
                },
            },
            Policy::Category(c) => match c.action {
                CategoryAction::Block => PolicyConfig::Block {
                    id,
                    app_id,
                    category_id,
                    active,
                },
                CategoryAction::TimeLimit {
                    limit_minutes,
                    extra_minutes,
                } => PolicyConfig::TimeLimit {
                    id,
                    app_id,
                    category_id,
                    time_limit_minutes: limit_minutes.max(1),
                    extra_minutes,
                    active,
                },
                CategoryAction::Notify {
                    limit_minutes,
                    repeat_interval_minutes,
                } => PolicyConfig::Notify {
                    id,
                    app_id,
                    category_id,
                    time_limit_minutes: limit_minutes.max(1),
                    notification_repeat_interval_minutes: repeat_interval_minutes,
                    active,
                },
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::Datelike;

    use super::*;

    #[test]
    fn test_time_limited_normal_remaining() {
        let app = TimeLimitedApp::Normal(50, 120);
        assert_eq!(app.remaining(), 70);
        assert!(app.can_extend());
        assert_eq!(app.effective_limit(), 120);
    }

    #[test]
    fn test_time_limited_extended_remaining() {
        let app = TimeLimitedApp::Extended(80, 120);
        assert_eq!(app.remaining(), 40);
        assert!(!app.can_extend());
        assert_eq!(app.effective_limit(), 120);
    }

    #[test]
    fn test_time_limited_exceeded() {
        let app = TimeLimitedApp::Normal(100, 60);
        assert_eq!(app.remaining(), -40);
        assert!(app.can_extend());
    }

    #[test]
    fn test_time_tracked_remaining_within_limit() {
        let app = TimeTrackedApp {
            used: 50,
            limit: 120,
        };
        assert_eq!(app.remaining(), 70);
        assert!(!app.is_exceeded());
    }

    #[test]
    fn test_time_tracked_exceeded() {
        let app = TimeTrackedApp {
            used: 100,
            limit: 60,
        };
        assert_eq!(app.remaining(), 0);
        assert!(app.is_exceeded());
    }

    #[test]
    fn test_time_tracked_at_limit() {
        let app = TimeTrackedApp {
            used: 60,
            limit: 60,
        };
        assert_eq!(app.remaining(), 0);
        assert!(app.is_exceeded());
    }

    #[test]
    fn test_time_window_empty_json_returns_none() {
        let tw: Option<TimeWindow> = serde_json::from_str("").unwrap_or_default();
        assert!(tw.is_none());
    }

    #[test]
    fn test_time_window_single_no_days() {
        let json = r#"[{"start_hour": 9, "end_hour": 17}]"#;
        let windows: Vec<TimeWindow> = serde_json::from_str(json).unwrap();
        let tw = windows.into_iter().next();
        assert!(tw.is_some());
        let tw = tw.unwrap();
        assert_eq!(tw.start_hour, 9);
        assert_eq!(tw.end_hour, 17);
        assert!(tw.days.is_empty());
    }

    #[test]
    fn test_time_window_with_days() {
        let json = r#"[{"start_hour": 9, "end_hour": 17, "days": [1,2,3,4,5]}]"#;
        let windows: Vec<TimeWindow> = serde_json::from_str(json).unwrap();
        let tw = windows.into_iter().next().unwrap();
        assert_eq!(tw.days, vec![1, 2, 3, 4, 5]);
    }

    #[test]
    fn test_time_window_no_windows_key() {
        let windows: Vec<TimeWindow> = serde_json::from_str(r#"[]"#).unwrap();
        assert!(windows.is_empty());
    }

    #[test]
    fn test_time_window_is_active_day_match() {
        let dt = chrono::DateTime::parse_from_rfc3339("2026-07-17T10:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        assert_eq!(dt.weekday().num_days_from_sunday(), 5);

        let w = TimeWindow {
            start_hour: 9,
            end_hour: 17,
            days: vec![5],
        };
        assert!(w.is_active(dt));
    }

    #[test]
    fn test_time_window_not_active_wrong_day() {
        let dt = chrono::DateTime::parse_from_rfc3339("2026-07-17T10:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);

        let w = TimeWindow {
            start_hour: 9,
            end_hour: 17,
            days: vec![1, 2, 3, 4],
        };
        assert!(!w.is_active(dt));
    }

    #[test]
    fn test_time_window_all_days_active() {
        let dt = chrono::DateTime::parse_from_rfc3339("2026-07-17T10:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);

        let w = TimeWindow {
            start_hour: 9,
            end_hour: 17,
            days: vec![],
        };
        assert!(w.is_active(dt));
    }

    #[test]
    fn test_time_window_midnight_wrap_active() {
        let dt = chrono::DateTime::parse_from_rfc3339("2026-07-17T23:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);

        let w = TimeWindow {
            start_hour: 22,
            end_hour: 2,
            days: vec![],
        };
        assert!(w.is_active(dt));
    }

    #[test]
    fn test_time_window_midnight_wrap_early() {
        let dt = chrono::DateTime::parse_from_rfc3339("2026-07-18T01:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);

        let w = TimeWindow {
            start_hour: 22,
            end_hour: 2,
            days: vec![],
        };
        assert!(w.is_active(dt));
    }

    #[test]
    fn test_time_window_midnight_wrap_outside() {
        let dt = chrono::DateTime::parse_from_rfc3339("2026-07-18T03:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);

        let w = TimeWindow {
            start_hour: 22,
            end_hour: 2,
            days: vec![],
        };
        assert!(!w.is_active(dt));
    }

    #[test]
    fn test_time_window_not_active_outside_hours() {
        let dt = chrono::DateTime::parse_from_rfc3339("2026-07-17T20:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);

        let w = TimeWindow {
            start_hour: 9,
            end_hour: 17,
            days: vec![],
        };
        assert!(!w.is_active(dt));
    }

    #[test]
    fn test_policy_config_from_domain_policy_full() {
        let meta = PolicyMeta {
            id: PolicyId(42),
            name: "Test".into(),
            time_windows: None,
            active: true,
            created_by: 1000,
            owner_id: 1000,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let p = Policy::App(Box::new(AppPolicy {
            target: AppTarget {
                app_id: AppId::new("firefox").unwrap(),
            },
            meta,
            action: AppAction::TimeLimit {
                limit_minutes: 3600,
                extra_minutes: 300,
            },
        }));

        let cfg: PolicyConfig = p.into();
        match &cfg {
            PolicyConfig::TimeLimit {
                id,
                app_id,
                category_id,
                time_limit_minutes,
                extra_minutes,
                active,
            } => {
                assert_eq!(*id, PolicyId(42));
                assert_eq!(app_id.as_ref().unwrap().as_str(), "firefox");
                assert!(category_id.is_none());
                assert_eq!(*time_limit_minutes, 3600);
                assert_eq!(*extra_minutes, 300);
                assert!(*active);
            }
            _ => panic!("expected TimeLimit"),
        }
    }

    #[test]
    fn test_policy_config_empty_app_id_and_sentinels() {
        let meta = PolicyMeta {
            id: PolicyId(1),
            name: "CatBlock".into(),
            time_windows: None,
            active: true,
            created_by: 1000,
            owner_id: 1000,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let p = Policy::Category(Box::new(CategoryPolicy {
            target: CategoryTarget {
                category_id: CategoryId(5),
            },
            meta,
            action: CategoryAction::Block,
        }));

        let cfg: PolicyConfig = p.into();
        match &cfg {
            PolicyConfig::Block {
                app_id,
                category_id,
                ..
            } => {
                assert!(app_id.is_none());
                assert_eq!(*category_id, Some(CategoryId(5)));
            }
            _ => panic!("expected Block"),
        }
    }
}
