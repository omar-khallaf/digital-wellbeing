//! Business logic for policy evaluation and state tracking.

use chrono::{DateTime, Utc};
use wellbeing_core::BlockReason;

use super::domain::*;

/// Compute the tracked state for an app given usage data and policy config.
pub fn app_state(usage: (i64, bool), policy: &PolicyConfig) -> TrackedApp {
    match policy {
        PolicyConfig::Block { .. } => {
            unreachable!("Block policy has no tracked state")
        }
        PolicyConfig::TimeLimit {
            time_limit_minutes,
            extra_minutes,
            ..
        } => {
            // usage.0 is already in minutes (converted from ms at policy boundary)
            let app = if usage.1 {
                TimeLimitedApp::Extended(usage.0, *time_limit_minutes + *extra_minutes)
            } else {
                TimeLimitedApp::Normal(usage.0, *time_limit_minutes)
            };
            TrackedApp::TimeLimited(app)
        }
        PolicyConfig::Notify {
            time_limit_minutes, ..
        } => TrackedApp::TimeTracked(TimeTrackedApp {
            used: usage.0,
            limit: *time_limit_minutes,
        }),
    }
}

fn eval_block_policy(policy: &PolicyConfig, first_block: bool) -> Option<PolicyVerdict> {
    if first_block {
        return None;
    }
    match policy {
        PolicyConfig::Block { app_id, id, .. } => {
            let reason = if app_id.is_some() {
                BlockReason::AppBlock
            } else {
                BlockReason::CategoryBlock
            };
            Some(PolicyVerdict::Block {
                policy_id: *id,
                reason,
                remaining: 0,
            })
        }
        _ => None,
    }
}

fn eval_timelimit_policy(
    policy: &PolicyConfig,
    elapsed_usage: i64,
    extended: bool,
    first_block: bool,
) -> Option<PolicyVerdict> {
    if first_block {
        return None;
    }
    match policy {
        PolicyConfig::TimeLimit {
            id,
            app_id,
            time_limit_minutes,
            extra_minutes,
            ..
        } => {
            let effective_limit_minutes = if extended {
                *time_limit_minutes + *extra_minutes
            } else {
                *time_limit_minutes
            };
            let remaining = effective_limit_minutes - elapsed_usage;
            if remaining <= 0 {
                let reason = if app_id.is_some() {
                    BlockReason::AppTimeLimit
                } else {
                    BlockReason::CategoryTimeLimit
                };
                Some(PolicyVerdict::Block {
                    policy_id: *id,
                    reason,
                    remaining,
                })
            } else {
                None
            }
        }
        _ => None,
    }
}

fn eval_notify_policy(
    policy: &PolicyConfig,
    elapsed_usage: i64,
    first_notify: bool,
) -> Option<PolicyVerdict> {
    if first_notify {
        return None;
    }
    match policy {
        PolicyConfig::Notify {
            id,
            time_limit_minutes,
            notification_repeat_interval_minutes,
            ..
        } => {
            let limit_minutes = *time_limit_minutes;
            if elapsed_usage >= limit_minutes {
                Some(PolicyVerdict::Notify {
                    policy_id: *id,
                    repeat_interval: *notification_repeat_interval_minutes,
                })
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Evaluate policies for an app and produce a verdict.
pub fn evaluate(policies: &[PolicyConfig], elapsed_usage: i64, extended: bool) -> PolicyVerdict {
    let mut first_block: Option<PolicyVerdict> = None;
    let mut first_notify: Option<PolicyVerdict> = None;

    for policy in policies {
        if !policy.active() {
            continue;
        }
        if let Some(block) = eval_block_policy(policy, first_block.is_some()).or_else(|| {
            eval_timelimit_policy(policy, elapsed_usage, extended, first_block.is_some())
        }) {
            first_block = Some(block);
        } else if let Some(notify) =
            eval_notify_policy(policy, elapsed_usage, first_notify.is_some())
        {
            first_notify = Some(notify);
        }
    }
    first_block.or(first_notify).unwrap_or(PolicyVerdict::Ok)
}

/// Filter policies by schedule, returning PolicyConfigs that are active now.
pub fn filter_policies_by_schedule(policies: Vec<Policy>, now: DateTime<Utc>) -> Vec<PolicyConfig> {
    policies
        .into_iter()
        .filter(|p| match p.time_windows() {
            None => true,
            Some(tw) => tw.is_active(now),
        })
        .map(PolicyConfig::from)
        .collect()
}

#[cfg(test)]
mod tests {
    use wellbeing_core::{AppId, CategoryId, PolicyId, TimeWindow};

    use super::*;

    // PolicyConfig test helpers removed — callers construct variants directly.

    #[test]
    fn test_evaluate_all_pass() {
        let policies = vec![
            PolicyConfig::TimeLimit {
                id: PolicyId(1),
                app_id: Some(AppId::new("test.app").unwrap()),
                category_id: None,
                time_limit_minutes: 60,
                extra_minutes: 300,
                active: true,
            },
            PolicyConfig::Notify {
                id: PolicyId(2),
                app_id: Some(AppId::new("test.app").unwrap()),
                category_id: None,
                time_limit_minutes: 120,
                notification_repeat_interval_minutes: None,
                active: true,
            },
        ];
        let verdict = evaluate(&policies, 50, false);
        assert!(matches!(verdict, PolicyVerdict::Ok));
    }

    #[test]
    fn test_evaluate_empty_policies() {
        let verdict = evaluate(&[], 0, false);
        assert!(matches!(verdict, PolicyVerdict::Ok));
    }

    #[test]
    fn test_evaluate_block_unconditional() {
        let policies = vec![PolicyConfig::Block {
            id: PolicyId(1),
            app_id: Some(AppId::new("test.app").unwrap()),
            category_id: None,
            active: true,
        }];
        let verdict = evaluate(&policies, 0, false);
        assert!(matches!(
            verdict,
            PolicyVerdict::Block {
                reason: BlockReason::AppBlock,
                ..
            }
        ));
    }

    #[test]
    fn test_evaluate_block_category() {
        let policies = vec![PolicyConfig::Block {
            id: PolicyId(1),
            app_id: None,
            category_id: Some(CategoryId(1)),
            active: true,
        }];
        let verdict = evaluate(&policies, 0, false);
        assert!(matches!(
            verdict,
            PolicyVerdict::Block {
                reason: BlockReason::CategoryBlock,
                ..
            }
        ));
    }

    #[test]
    fn test_evaluate_time_limit_block() {
        let policies = vec![PolicyConfig::TimeLimit {
            id: PolicyId(1),
            app_id: Some(AppId::new("test.app").unwrap()),
            category_id: None,
            time_limit_minutes: 60,
            extra_minutes: 300,
            active: true,
        }];
        let verdict = evaluate(&policies, 4000, false);
        assert!(matches!(
            verdict,
            PolicyVerdict::Block {
                reason: BlockReason::AppTimeLimit,
                ..
            }
        ));
    }

    #[test]
    fn test_evaluate_time_limit_category_block() {
        let policies = vec![PolicyConfig::TimeLimit {
            id: PolicyId(1),
            app_id: None,
            category_id: Some(CategoryId(1)),
            time_limit_minutes: 60,
            extra_minutes: 300,
            active: true,
        }];
        let verdict = evaluate(&policies, 4000, false);
        assert!(matches!(
            verdict,
            PolicyVerdict::Block {
                reason: BlockReason::CategoryTimeLimit,
                ..
            }
        ));
    }

    #[test]
    fn test_evaluate_notify_exceeded() {
        let policies = vec![PolicyConfig::Notify {
            id: PolicyId(1),
            app_id: Some(AppId::new("test.app").unwrap()),
            category_id: None,
            time_limit_minutes: 60,
            notification_repeat_interval_minutes: None,
            active: true,
        }];
        let verdict = evaluate(&policies, 4000, false);
        assert!(matches!(verdict, PolicyVerdict::Notify { .. }));
    }

    #[test]
    fn test_evaluate_block_wins_over_notify() {
        let policies = vec![
            PolicyConfig::Notify {
                id: PolicyId(1),
                app_id: Some(AppId::new("test.app").unwrap()),
                category_id: None,
                time_limit_minutes: 60,
                notification_repeat_interval_minutes: None,
                active: true,
            },
            PolicyConfig::Block {
                id: PolicyId(2),
                app_id: Some(AppId::new("test.app").unwrap()),
                category_id: None,
                active: true,
            },
        ];
        let verdict = evaluate(&policies, 4000, false);
        assert!(matches!(verdict, PolicyVerdict::Block { .. }));
    }

    #[test]
    fn test_evaluate_block_wins_over_time_limit() {
        let policies = vec![
            PolicyConfig::Block {
                id: PolicyId(1),
                app_id: Some(AppId::new("test.app").unwrap()),
                category_id: None,
                active: true,
            },
            PolicyConfig::TimeLimit {
                id: PolicyId(2),
                app_id: Some(AppId::new("test.app").unwrap()),
                category_id: None,
                time_limit_minutes: 100,
                extra_minutes: 300,
                active: true,
            },
        ];
        let verdict = evaluate(&policies, 0, false);
        assert!(matches!(verdict, PolicyVerdict::Block { .. }));
    }

    #[test]
    fn test_evaluate_first_block_wins() {
        let policies = vec![
            PolicyConfig::TimeLimit {
                id: PolicyId(1),
                app_id: Some(AppId::new("test.app").unwrap()),
                category_id: None,
                time_limit_minutes: 2,
                extra_minutes: 300,
                active: true,
            },
            PolicyConfig::Block {
                id: PolicyId(2),
                app_id: None,
                category_id: Some(CategoryId(1)),
                active: true,
            },
        ];
        let verdict = evaluate(&policies, 200, false);
        assert!(
            matches!(verdict, PolicyVerdict::Block { policy_id, .. } if policy_id == PolicyId(1))
        );
    }

    #[test]
    fn test_evaluate_inactive_policy_skipped() {
        let p = PolicyConfig::Block {
            id: PolicyId(1),
            app_id: Some(AppId::new("test.app").unwrap()),
            category_id: None,
            active: true,
        };
        let policies = match p {
            PolicyConfig::Block {
                id,
                app_id,
                category_id,
                ..
            } => {
                vec![PolicyConfig::Block {
                    id,
                    app_id,
                    category_id,
                    active: false,
                }]
            }
            _ => unreachable!(),
        };
        let verdict = evaluate(&policies, 0, false);
        assert!(matches!(verdict, PolicyVerdict::Ok));
    }

    #[test]
    fn test_evaluate_notify_with_repeat() {
        let policies = vec![PolicyConfig::Notify {
            id: PolicyId(1),
            app_id: Some(AppId::new("test.app").unwrap()),
            category_id: None,
            time_limit_minutes: 60,
            notification_repeat_interval_minutes: Some(5),
            active: true,
        }];
        let verdict = evaluate(&policies, 60, false);
        assert!(matches!(
            verdict,
            PolicyVerdict::Notify {
                repeat_interval: Some(5),
                ..
            }
        ));
    }

    #[test]
    fn test_evaluate_time_limit_at_exact_boundary() {
        let policies = vec![PolicyConfig::TimeLimit {
            id: PolicyId(1),
            app_id: Some(AppId::new("test.app").unwrap()),
            category_id: None,
            time_limit_minutes: 60,
            extra_minutes: 300,
            active: true,
        }];
        // elapsed_usage in minutes: 60 minutes used, limit 60 minutes → blocked
        let verdict = evaluate(&policies, 60, false);
        assert!(matches!(verdict, PolicyVerdict::Block { .. }));
    }

    #[test]
    fn test_evaluate_notify_at_exact_boundary() {
        let policies = vec![PolicyConfig::Notify {
            id: PolicyId(1),
            app_id: Some(AppId::new("test.app").unwrap()),
            category_id: None,
            time_limit_minutes: 60,
            notification_repeat_interval_minutes: None,
            active: true,
        }];
        // elapsed_usage in minutes: 60 minutes used, limit 60 minutes → notify
        let verdict = evaluate(&policies, 60, false);
        assert!(matches!(verdict, PolicyVerdict::Notify { .. }));
    }

    #[test]
    #[should_panic(expected = "Block policy has no tracked state")]
    fn test_app_state_block_panics() {
        let policy = PolicyConfig::Block {
            id: PolicyId(1),
            app_id: Some(AppId::new("test.app").unwrap()),
            category_id: None,
            active: true,
        };
        app_state((0, false), &policy);
    }

    #[test]
    fn test_app_state_time_limit_normal() {
        let policy = PolicyConfig::TimeLimit {
            id: PolicyId(1),
            app_id: Some(AppId::new("test.app").unwrap()),
            category_id: None,
            time_limit_minutes: 60,
            extra_minutes: 300,
            active: true,
        };
        // usage in minutes: 30 min used, 60 min limit → 30 min remaining
        let state = app_state((30, false), &policy);
        match state {
            TrackedApp::TimeLimited(app) => {
                assert_eq!(app.remaining(), 30);
                assert!(app.can_extend());
                assert_eq!(app.effective_limit(), 60);
            }
            _ => panic!("expected TimeLimited"),
        }
    }

    #[test]
    fn test_app_state_time_limit_extended() {
        let policy = PolicyConfig::TimeLimit {
            id: PolicyId(1),
            app_id: Some(AppId::new("test.app").unwrap()),
            category_id: None,
            time_limit_minutes: 60,
            extra_minutes: 10,
            active: true,
        };
        // usage in minutes: 50 min used, 70 min effective limit → 20 min remaining
        let state = app_state((50, true), &policy);
        match state {
            TrackedApp::TimeLimited(app) => {
                assert_eq!(app.remaining(), 20);
                assert!(!app.can_extend());
                assert_eq!(app.effective_limit(), 70);
            }
            _ => panic!("expected TimeLimited"),
        }
    }

    #[test]
    fn test_app_state_notify() {
        let policy = PolicyConfig::Notify {
            id: PolicyId(1),
            app_id: Some(AppId::new("test.app").unwrap()),
            category_id: None,
            time_limit_minutes: 60,
            notification_repeat_interval_minutes: None,
            active: true,
        };
        // usage in minutes: 30 min used, 60 min limit → 30 min remaining
        let state = app_state((30, false), &policy);
        match state {
            TrackedApp::TimeTracked(app) => {
                assert_eq!(app.remaining(), 30);
                assert!(!app.is_exceeded());
            }
            _ => panic!("expected TimeTracked"),
        }
    }

    #[test]
    fn test_filter_policies_empty_schedule_kept() {
        let p = Policy::App(Box::new(AppPolicy {
            target: AppTarget {
                app_id: AppId::new("test").unwrap(),
            },
            meta: PolicyMeta {
                id: PolicyId(1),
                name: "AlwaysActive".into(),
                time_windows: None,
                active: true,
                created_by: 1000,
                owner_id: 1000,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            },
            action: AppAction::TimeLimit {
                limit_minutes: 3600,
                extra_minutes: 0,
            },
        }));
        let result = filter_policies_by_schedule(vec![p], Utc::now());
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn test_filter_policies_with_schedule_active() {
        let p = Policy::App(Box::new(AppPolicy {
            target: AppTarget {
                app_id: AppId::new("test").unwrap(),
            },
            meta: PolicyMeta {
                id: PolicyId(1),
                name: "Scheduled".into(),
                time_windows: Some(TimeWindow {
                    start_hour: 0,
                    end_hour: 23,
                    days: vec![],
                }),
                active: true,
                created_by: 1000,
                owner_id: 1000,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            },
            action: AppAction::Block,
        }));
        let result = filter_policies_by_schedule(vec![p], Utc::now());
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn test_filter_policies_with_schedule_inactive() {
        let p = Policy::App(Box::new(AppPolicy {
            target: AppTarget {
                app_id: AppId::new("test").unwrap(),
            },
            meta: PolicyMeta {
                id: PolicyId(1),
                name: "NightOnly".into(),
                time_windows: Some(TimeWindow {
                    start_hour: 0,
                    end_hour: 1,
                    days: vec![],
                }),
                active: true,
                created_by: 1000,
                owner_id: 1000,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            },
            action: AppAction::Block,
        }));
        let now = chrono::DateTime::parse_from_rfc3339("2026-07-17T10:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let result = filter_policies_by_schedule(vec![p], now);
        assert_eq!(result.len(), 0);
    }
}
