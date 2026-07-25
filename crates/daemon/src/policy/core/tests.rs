use chrono::Utc;
use wellbeing_core::{AppId, CategoryId, PolicyId, TimeWindow};

use super::*;
use crate::policy::domain::*;

#[test]
fn test_evaluate_all_pass() {
    let policies = vec![
        PolicyConfig::TimeLimit {
            id: PolicyId(1),
            app_id: Some(AppId::new("test.app").unwrap()),
            category_id: None,
            time_limit_minutes: 60,
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
    let verdict = evaluate(&policies, 50);
    assert!(matches!(verdict, PolicyVerdict::Ok));
}

#[test]
fn test_evaluate_empty_policies() {
    let verdict = evaluate(&[], 0);
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
    let verdict = evaluate(&policies, 0);
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
    let verdict = evaluate(&policies, 0);
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
        active: true,
    }];
    let verdict = evaluate(&policies, 4000);
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
        active: true,
    }];
    let verdict = evaluate(&policies, 4000);
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
    let verdict = evaluate(&policies, 4000);
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
    let verdict = evaluate(&policies, 4000);
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
            active: true,
        },
    ];
    let verdict = evaluate(&policies, 0);
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
            active: true,
        },
        PolicyConfig::Block {
            id: PolicyId(2),
            app_id: None,
            category_id: Some(CategoryId(1)),
            active: true,
        },
    ];
    let verdict = evaluate(&policies, 200);
    assert!(matches!(verdict, PolicyVerdict::Block { policy_id, .. } if policy_id == PolicyId(1)));
}

#[test]
fn test_evaluate_inactive_policy_skipped() {
    let policies = vec![PolicyConfig::Block {
        id: PolicyId(1),
        app_id: Some(AppId::new("test.app").unwrap()),
        category_id: None,
        active: false,
    }];
    let verdict = evaluate(&policies, 0);
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
    let verdict = evaluate(&policies, 60);
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
        active: true,
    }];
    let verdict = evaluate(&policies, 60);
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
    let verdict = evaluate(&policies, 60);
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
    app_state(0, &policy);
}

#[test]
fn test_app_state_time_limit_normal() {
    let policy = PolicyConfig::TimeLimit {
        id: PolicyId(1),
        app_id: Some(AppId::new("test.app").unwrap()),
        category_id: None,
        time_limit_minutes: 60,
        active: true,
    };
    let state = app_state(30, &policy);
    match state {
        TrackedApp::TimeLimited(app) => {
            assert_eq!(app.remaining(), 30);
        }
        _ => panic!("expected TimeLimited"),
    }
}

#[test]
fn test_app_state_time_limit_normal_base_used() {
    let policy = PolicyConfig::TimeLimit {
        id: PolicyId(1),
        app_id: Some(AppId::new("test.app").unwrap()),
        category_id: None,
        time_limit_minutes: 60,
        active: true,
    };
    let state = app_state(50, &policy);
    match state {
        TrackedApp::TimeLimited(app) => {
            assert_eq!(app.remaining(), 10);
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
    let state = app_state(30, &policy);
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
