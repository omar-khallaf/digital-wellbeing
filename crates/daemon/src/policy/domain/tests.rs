use wellbeing_core::{PolicyId, TimeWindow};

use super::*;

#[test]
fn test_effect_terminating_semantics() {
    assert!(Effect::Allow.is_terminating());
    assert!(Effect::Block.is_terminating());
    assert!(Effect::TimeLimit { limit_minutes: 30 }.is_terminating());
    assert!(!Effect::Notify { limit_minutes: 30 }.is_terminating());
}

#[test]
fn test_effect_kind_discriminant() {
    assert_eq!(
        Effect::Allow.kind_discriminant(),
        wellbeing_core::Effect::Allow
    );
    assert_eq!(
        Effect::Block.kind_discriminant(),
        wellbeing_core::Effect::Block
    );
    assert_eq!(
        Effect::TimeLimit { limit_minutes: 30 }.kind_discriminant(),
        wellbeing_core::Effect::TimeLimit
    );
    assert_eq!(
        Effect::Notify { limit_minutes: 30 }.kind_discriminant(),
        wellbeing_core::Effect::Notify
    );
}

#[test]
fn test_eval_result_is_unrestricted() {
    let r = EvalResult {
        terminating: None,
        notifies: vec![],
    };
    assert!(r.is_unrestricted());

    let r = EvalResult {
        terminating: Some((PolicyId(1), Effect::Block)),
        notifies: vec![],
    };
    assert!(!r.is_unrestricted());

    let r = EvalResult {
        terminating: None,
        notifies: vec![(PolicyId(1), Effect::Notify { limit_minutes: 30 })],
    };
    assert!(!r.is_unrestricted());
}

#[test]
fn test_time_window_day_mask_is_active() {
    // Friday 2026-07-17 = day 5 from Sunday = bit 5 = 0b00100000 = 32
    let dt = chrono::DateTime::parse_from_rfc3339("2026-07-17T10:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);

    let w = TimeWindow {
        start_minute: 540,
        end_minute: 1020,
        day_mask: 0b00100000,
    }; // 09:00–17:00
    assert!(w.is_active(dt));

    let w = TimeWindow {
        start_minute: 540,
        end_minute: 1020,
        day_mask: 0b00010000,
    };
    assert!(!w.is_active(dt)); // Thursday bit not set
}

#[test]
fn test_time_window_day_mask_zero_always_active() {
    let dt = chrono::DateTime::parse_from_rfc3339("2026-07-17T10:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);

    let w = TimeWindow {
        start_minute: 540,
        end_minute: 1020,
        day_mask: 0,
    };
    assert!(w.is_active(dt));
}

#[test]
fn test_time_window_cross_midnight() {
    let dt = chrono::DateTime::parse_from_rfc3339("2026-07-17T23:30:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    let w = TimeWindow {
        start_minute: 1320,
        end_minute: 360,
        day_mask: 0,
    }; // 22:00–06:00
    assert!(w.is_active(dt));

    let dt2 = chrono::DateTime::parse_from_rfc3339("2026-07-18T03:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    assert!(w.is_active(dt2));

    let dt3 = chrono::DateTime::parse_from_rfc3339("2026-07-18T07:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    assert!(!w.is_active(dt3));
}
