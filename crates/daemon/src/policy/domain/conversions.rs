use super::types::*;
use chrono::Utc;

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
                AppAction::TimeLimit { limit_minutes } => PolicyConfig::TimeLimit {
                    id,
                    app_id,
                    category_id,
                    time_limit_minutes: limit_minutes.max(1),
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
                CategoryAction::TimeLimit { limit_minutes } => PolicyConfig::TimeLimit {
                    id,
                    app_id,
                    category_id,
                    time_limit_minutes: limit_minutes.max(1),
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
