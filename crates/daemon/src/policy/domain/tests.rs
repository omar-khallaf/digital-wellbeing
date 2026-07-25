use super::*;
use chrono::{Datelike, Utc};
use wellbeing_core::TimeWindow;

#[test]
fn test_time_limited_remaining() {
    let app = TimeLimitedApp {
        used: 50,
        limit: 120,
    };
    assert_eq!(app.remaining(), 70);
}

#[test]
fn test_time_limited_exceeded() {
    let app = TimeLimitedApp {
        used: 100,
        limit: 60,
    };
    assert_eq!(app.remaining(), -40);
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
        },
    }));

    let cfg: PolicyConfig = p.into();
    match &cfg {
        PolicyConfig::TimeLimit {
            id,
            app_id,
            category_id,
            time_limit_minutes,
            active,
        } => {
            assert_eq!(*id, PolicyId(42));
            assert_eq!(app_id.as_ref().unwrap().as_str(), "firefox");
            assert!(category_id.is_none());
            assert_eq!(*time_limit_minutes, 3600);
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
