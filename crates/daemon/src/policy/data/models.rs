//! Diesel Queryable structs for the policies and daily_usage tables.

use chrono::Utc;
use wellbeing_core::{AppId, CategoryId, PolicyId, TimeWindow};

use crate::policy;

/// Row type for the `policies` table.
#[derive(Debug, Clone, diesel::Queryable)]
pub(crate) struct PolicyRow {
    pub(crate) id: i32,
    pub(crate) name: String,
    pub(crate) action: i32,
    pub(crate) category_id: Option<i32>,
    pub(crate) app_id: Option<String>,
    pub(crate) created_by: i32,
    pub(crate) owner_id: i32,
    pub(crate) time_limit_minutes: Option<i32>,
    pub(crate) extra_minutes: i32,
    pub(crate) notification_repeat_interval_minutes: Option<i32>,
    pub(crate) schedule_start_hour: Option<i32>,
    pub(crate) schedule_end_hour: Option<i32>,
    pub(crate) schedule_days: String,
    pub(crate) active: bool,
    pub(crate) created_at: String,
    pub(crate) updated_at: String,
}

impl PolicyRow {
    pub(crate) fn into_domain_policy(self) -> policy::Policy {
        let tlm = self.time_limit_minutes.map_or(1, |v| (v as i64).max(1));
        let extra = self.extra_minutes as i64;
        let repeat = self
            .notification_repeat_interval_minutes
            .map(|v| v as i64)
            .filter(|v| *v > 0);

        let meta = policy::PolicyMeta {
            id: PolicyId(self.id as i64),
            name: self.name,
            time_windows: match (self.schedule_start_hour, self.schedule_end_hour) {
                (Some(start), Some(end)) => {
                    let days: Vec<u8> =
                        serde_json::from_str(&self.schedule_days).unwrap_or_default();
                    Some(TimeWindow {
                        start_hour: start as u8,
                        end_hour: end as u8,
                        days,
                    })
                }
                _ => None,
            },
            active: self.active,
            created_by: self.created_by as u32,
            owner_id: self.owner_id as u32,
            created_at: self.created_at.parse().ok().unwrap_or_else(Utc::now),
            updated_at: self.updated_at.parse().ok().unwrap_or_else(Utc::now),
        };

        match (self.action, self.app_id) {
            (0, Some(aid)) => policy::Policy::App(Box::new(policy::AppPolicy {
                target: policy::AppTarget {
                    app_id: AppId::new(&aid).unwrap_or_else(|_| AppId::new("unknown").unwrap()),
                },
                meta,
                action: policy::AppAction::Block,
            })),
            (1, Some(aid)) => policy::Policy::App(Box::new(policy::AppPolicy {
                target: policy::AppTarget {
                    app_id: AppId::new(&aid).unwrap_or_else(|_| AppId::new("unknown").unwrap()),
                },
                meta,
                action: policy::AppAction::TimeLimit {
                    limit_minutes: tlm,
                    extra_minutes: extra,
                },
            })),
            (2, Some(aid)) => policy::Policy::App(Box::new(policy::AppPolicy {
                target: policy::AppTarget {
                    app_id: AppId::new(&aid).unwrap_or_else(|_| AppId::new("unknown").unwrap()),
                },
                meta,
                action: policy::AppAction::Notify {
                    limit_minutes: tlm,
                    repeat_interval_minutes: repeat,
                },
            })),
            (0, None) => policy::Policy::Category(Box::new(policy::CategoryPolicy {
                target: policy::CategoryTarget {
                    category_id: CategoryId(self.category_id.unwrap_or(0) as i64),
                },
                meta,
                action: policy::CategoryAction::Block,
            })),
            (1, None) => policy::Policy::Category(Box::new(policy::CategoryPolicy {
                target: policy::CategoryTarget {
                    category_id: CategoryId(self.category_id.unwrap_or(0) as i64),
                },
                meta,
                action: policy::CategoryAction::TimeLimit {
                    limit_minutes: tlm,
                    extra_minutes: extra,
                },
            })),
            (2, None) => policy::Policy::Category(Box::new(policy::CategoryPolicy {
                target: policy::CategoryTarget {
                    category_id: CategoryId(self.category_id.unwrap_or(0) as i64),
                },
                meta,
                action: policy::CategoryAction::Notify {
                    limit_minutes: tlm,
                    repeat_interval_minutes: repeat,
                },
            })),
            _ => {
                tracing::error!(
                    action = self.action,
                    "invalid policy action, defaulting to Block"
                );
                policy::Policy::Category(Box::new(policy::CategoryPolicy {
                    target: policy::CategoryTarget {
                        category_id: CategoryId(self.category_id.unwrap_or(0) as i64),
                    },
                    meta,
                    action: policy::CategoryAction::Block,
                }))
            }
        }
    }
}

/// Row type for the `daily_usage` table — used to replace raw tuples.
#[derive(Debug, Clone, diesel::Queryable)]
pub(crate) struct DailyUsageRow {
    pub(crate) date: String,
    pub(crate) user_id: i32,
    pub(crate) app_id: String,
    pub(crate) closed_millis: i32,
    pub(crate) open_millis: i32,
    pub(crate) extended: bool,
}
