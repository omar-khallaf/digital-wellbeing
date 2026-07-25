use chrono::{DateTime, Utc};
use wellbeing_core::TimeWindow;
pub use wellbeing_core::{AppId, BlockReason, CategoryId, PolicyId};

pub struct TimeLimitedApp {
    pub used: i64,
    pub limit: i64,
}

impl TimeLimitedApp {
    pub fn remaining(&self) -> i64 {
        self.limit - self.used
    }
}

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

#[derive(Debug, Clone)]
pub struct AppTarget {
    pub app_id: AppId,
}

#[derive(Debug, Clone)]
pub struct CategoryTarget {
    pub category_id: CategoryId,
}

#[derive(Debug, Clone)]
pub enum AppAction {
    Block,
    TimeLimit {
        limit_minutes: i64,
    },
    Notify {
        limit_minutes: i64,
        repeat_interval_minutes: Option<i64>,
    },
}

#[derive(Debug, Clone)]
pub enum CategoryAction {
    Block,
    TimeLimit {
        limit_minutes: i64,
    },
    Notify {
        limit_minutes: i64,
        repeat_interval_minutes: Option<i64>,
    },
}

#[derive(Debug, Clone)]
pub struct AppPolicy {
    pub target: AppTarget,
    pub meta: PolicyMeta,
    pub action: AppAction,
}

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
