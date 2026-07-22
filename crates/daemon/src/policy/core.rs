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
            let app = if usage.1 {
                TimeLimitedApp::Extended(usage.0, (*time_limit_minutes + *extra_minutes) * 60)
            } else {
                TimeLimitedApp::Normal(usage.0, *time_limit_minutes * 60)
            };
            TrackedApp::TimeLimited(app)
        }
        PolicyConfig::Notify {
            time_limit_minutes, ..
        } => TrackedApp::TimeTracked(TimeTrackedApp {
            used: usage.0,
            limit: *time_limit_minutes * 60,
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
    use wellbeing_core::{AppId, CategoryId, PolicyId, PolicyKind, TimeWindow};

    use super::*;

    fn make_policy(id: i64, kind: PolicyKind, app: bool, limit: Option<i64>) -> PolicyConfig {
        let app_id = if app {
            Some(AppId::new("test.app").unwrap())
        } else {
            None
        };
        let category_id = if app { None } else { Some(CategoryId(1)) };
        match kind {
            PolicyKind::Block => PolicyConfig::Block {
                id: PolicyId(id),
                app_id,
                category_id,
                active: true,
            },
            PolicyKind::TimeLimit => PolicyConfig::TimeLimit {
                id: PolicyId(id),
                app_id,
                category_id,
                time_limit_minutes: limit.unwrap_or(60),
                extra_minutes: 300,
                active: true,
            },
            PolicyKind::Notify => PolicyConfig::Notify {
                id: PolicyId(id),
                app_id,
                category_id,
                time_limit_minutes: limit.unwrap_or(60),
                notification_repeat_interval_minutes: None,
                active: true,
            },
        }
    }

    fn make_policy_full(
        id: i64,
        kind: PolicyKind,
        app: bool,
        limit: Option<i64>,
        extra: i64,
        repeat: Option<i64>,
        active: bool,
    ) -> PolicyConfig {
        let app_id = if app {
            Some(AppId::new("test.app").unwrap())
        } else {
            None
        };
        let category_id = if app { None } else { Some(CategoryId(1)) };
        match kind {
            PolicyKind::Block => PolicyConfig::Block {
                id: PolicyId(id),
                app_id,
                category_id,
                active,
            },
            PolicyKind::TimeLimit => PolicyConfig::TimeLimit {
                id: PolicyId(id),
                app_id,
                category_id,
                time_limit_minutes: limit.unwrap_or(60),
                extra_minutes: extra,
                active,
            },
            PolicyKind::Notify => PolicyConfig::Notify {
                id: PolicyId(id),
                app_id,
                category_id,
                time_limit_minutes: limit.unwrap_or(60),
                notification_repeat_interval_minutes: repeat,
                active,
            },
        }
    }

    fn make_domain_policy(
        id: i64,
        name: &str,
        action: PolicyKind,
        app_id: &str,
        cat_id: i64,
        limit: i64,
        extra: i64,
        repeat: i64,
        schedule_start_hour: Option<u8>,
        schedule_end_hour: Option<u8>,
        schedule_days: &[u8],
        active: bool,
    ) -> Policy {
        let time_windows = match (schedule_start_hour, schedule_end_hour) {
            (Some(start), Some(end)) => Some(TimeWindow {
                start_hour: start,
                end_hour: end,
                days: schedule_days.to_vec(),
            }),
            _ => None,
        };
        let meta = PolicyMeta {
            id: PolicyId(id),
            name: name.into(),
            time_windows,
            active,
            created_by: 1000,
            owner_id: 1000,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        match (action, app_id.is_empty()) {
            (PolicyKind::Block, false) => Policy::App(Box::new(AppPolicy {
                target: AppTarget {
                    app_id: AppId::new(app_id).unwrap(),
                },
                meta,
                action: AppAction::Block,
            })),
            (PolicyKind::TimeLimit, false) => Policy::App(Box::new(AppPolicy {
                target: AppTarget {
                    app_id: AppId::new(app_id).unwrap(),
                },
                meta,
                action: AppAction::TimeLimit {
                    limit_minutes: limit.max(1),
                    extra_minutes: extra,
                },
            })),
            (PolicyKind::Notify, false) => Policy::App(Box::new(AppPolicy {
                target: AppTarget {
                    app_id: AppId::new(app_id).unwrap(),
                },
                meta,
                action: AppAction::Notify {
                    limit_minutes: limit.max(1),
                    repeat_interval_minutes: if repeat == 0 { None } else { Some(repeat) },
                },
            })),
            (PolicyKind::Block, true) => Policy::Category(Box::new(CategoryPolicy {
                target: CategoryTarget {
                    category_id: CategoryId(cat_id),
                },
                meta,
                action: CategoryAction::Block,
            })),
            (PolicyKind::TimeLimit, true) => Policy::Category(Box::new(CategoryPolicy {
                target: CategoryTarget {
                    category_id: CategoryId(cat_id),
                },
                meta,
                action: CategoryAction::TimeLimit {
                    limit_minutes: limit.max(1),
                    extra_minutes: extra,
                },
            })),
            (PolicyKind::Notify, true) => Policy::Category(Box::new(CategoryPolicy {
                target: CategoryTarget {
                    category_id: CategoryId(cat_id),
                },
                meta,
                action: CategoryAction::Notify {
                    limit_minutes: limit.max(1),
                    repeat_interval_minutes: if repeat == 0 { None } else { Some(repeat) },
                },
            })),
        }
    }

    #[test]
    fn test_evaluate_all_pass() {
        let policies = vec![
            make_policy(1, PolicyKind::TimeLimit, true, Some(60)),
            make_policy(2, PolicyKind::Notify, true, Some(120)),
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
        let policies = vec![make_policy(1, PolicyKind::Block, true, None)];
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
        let policies = vec![make_policy(1, PolicyKind::Block, false, None)];
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
        let policies = vec![make_policy(1, PolicyKind::TimeLimit, true, Some(60))];
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
        let policies = vec![make_policy(1, PolicyKind::TimeLimit, false, Some(60))];
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
        let policies = vec![make_policy(1, PolicyKind::Notify, true, Some(60))];
        let verdict = evaluate(&policies, 4000, false);
        assert!(matches!(verdict, PolicyVerdict::Notify { .. }));
    }

    #[test]
    fn test_evaluate_block_wins_over_notify() {
        let policies = vec![
            make_policy(1, PolicyKind::Notify, true, Some(60)),
            make_policy(2, PolicyKind::Block, true, None),
        ];
        let verdict = evaluate(&policies, 4000, false);
        assert!(matches!(verdict, PolicyVerdict::Block { .. }));
    }

    #[test]
    fn test_evaluate_block_wins_over_time_limit() {
        let policies = vec![
            make_policy(1, PolicyKind::Block, true, None),
            make_policy(2, PolicyKind::TimeLimit, true, Some(100)),
        ];
        let verdict = evaluate(&policies, 0, false);
        assert!(matches!(verdict, PolicyVerdict::Block { .. }));
    }

    #[test]
    fn test_evaluate_first_block_wins() {
        let policies = vec![
            make_policy(1, PolicyKind::TimeLimit, true, Some(2)),
            make_policy(2, PolicyKind::Block, false, None),
        ];
        let verdict = evaluate(&policies, 200, false);
        assert!(
            matches!(verdict, PolicyVerdict::Block { policy_id, .. } if policy_id == PolicyId(1))
        );
    }

    #[test]
    fn test_evaluate_inactive_policy_skipped() {
        let p = make_policy(1, PolicyKind::Block, true, None);
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
        let policies = vec![make_policy_full(
            1,
            PolicyKind::Notify,
            true,
            Some(60),
            0,
            Some(5),
            true,
        )];
        let verdict = evaluate(&policies, 4000, false);
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
        let policies = vec![make_policy(1, PolicyKind::TimeLimit, true, Some(60))];
        let verdict = evaluate(&policies, 3600, false);
        assert!(matches!(verdict, PolicyVerdict::Block { .. }));
    }

    #[test]
    fn test_evaluate_notify_at_exact_boundary() {
        let policies = vec![make_policy(1, PolicyKind::Notify, true, Some(60))];
        let verdict = evaluate(&policies, 3600, false);
        assert!(matches!(verdict, PolicyVerdict::Notify { .. }));
    }

    #[test]
    #[should_panic(expected = "Block policy has no tracked state")]
    fn test_app_state_block_panics() {
        let policy = make_policy(1, PolicyKind::Block, true, None);
        app_state((0, false), &policy);
    }

    #[test]
    fn test_app_state_time_limit_normal() {
        let policy = make_policy(1, PolicyKind::TimeLimit, true, Some(60));
        let state = app_state((100, false), &policy);
        match state {
            TrackedApp::TimeLimited(app) => {
                assert_eq!(app.remaining(), 3500);
                assert!(app.can_extend());
                assert_eq!(app.effective_limit(), 3600);
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
        let state = app_state((4000, true), &policy);
        match state {
            TrackedApp::TimeLimited(app) => {
                assert_eq!(app.remaining(), 200);
                assert!(!app.can_extend());
                assert_eq!(app.effective_limit(), 4200);
            }
            _ => panic!("expected TimeLimited"),
        }
    }

    #[test]
    fn test_app_state_notify() {
        let policy = make_policy(1, PolicyKind::Notify, true, Some(60));
        let state = app_state((100, false), &policy);
        match state {
            TrackedApp::TimeTracked(app) => {
                assert_eq!(app.remaining(), 3500);
                assert!(!app.is_exceeded());
            }
            _ => panic!("expected TimeTracked"),
        }
    }

    #[test]
    fn test_filter_policies_empty_schedule_kept() {
        let p = make_domain_policy(
            1,
            "AlwaysActive",
            PolicyKind::TimeLimit,
            "test",
            0,
            3600,
            0,
            0,
            None,
            None,
            &[],
            true,
        );
        let result = filter_policies_by_schedule(vec![p], Utc::now());
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn test_filter_policies_with_schedule_active() {
        let p = make_domain_policy(
            1,
            "Scheduled",
            PolicyKind::Block,
            "test",
            0,
            0,
            0,
            0,
            Some(0),
            Some(23),
            &[],
            true,
        );
        let result = filter_policies_by_schedule(vec![p], Utc::now());
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn test_filter_policies_with_schedule_inactive() {
        let p = make_domain_policy(
            1,
            "NightOnly",
            PolicyKind::Block,
            "test",
            0,
            0,
            0,
            0,
            Some(0),
            Some(1),
            &[],
            true,
        );
        let now = chrono::DateTime::parse_from_rfc3339("2026-07-17T10:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let result = filter_policies_by_schedule(vec![p], now);
        assert_eq!(result.len(), 0);
    }
}
