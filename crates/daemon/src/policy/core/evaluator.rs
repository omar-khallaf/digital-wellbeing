//! Policy evaluation engine — priority-ordered first-match-wins.
//!
//! `evaluate()` is a pure domain function that accepts a sorted slice of
//! policies and returns an `EvalResult`. It does no I/O — all data must
//! be fetched and filtered beforehand.
//!
//! Evaluation rules:
//! - Policies are already sorted by priority (ascending) by the SQL query.
//! - `Allow` / `Block` / `TimeLimit` are **terminating**: the first match
//!   wins and evaluation stops.
//! - `Notify` is **non-terminating**: appended to `notifies`, evaluation
//!   continues.
//! - `Target::Any` matches everything but is evaluated last in priority order.
//! - Empty slice → `EvalResult::unrestricted()`.

use tracing::debug;
use wellbeing_core::PolicyId;

use crate::policy::domain::{Effect, EvalResult, Policy};

/// Check if a schedule (vec of time windows) is active at `now`.
///
/// An empty schedule (= no windows) is always active. A non-empty schedule
/// matches if **any** window is active (OR semantics). Cross-midnight windows
/// are handled by [`TimeWindow::is_active`].
pub fn filter_by_schedule(
    schedule: &[wellbeing_core::TimeWindow],
    now: chrono::DateTime<chrono::Utc>,
) -> bool {
    if schedule.is_empty() {
        return true;
    }
    schedule.iter().any(|tw| tw.is_active(now))
}

/// Evaluate policies for an app given its usage and the current time.
///
/// `policies` **must** be pre-sorted by `priority` ascending.
/// `usage_minutes` is the accumulated usage for the current day.
///
/// Returns `EvalResult` with:
/// - `terminating`: the first Allow/Block/TimeLimit that matched (if any).
/// - `notifies`: all matching Notify effects, in priority order.
pub fn evaluate(
    policies: &[Policy],
    usage_minutes: u64,
    now: chrono::DateTime<chrono::Utc>,
) -> EvalResult {
    let mut terminating: Option<(PolicyId, Effect)> = None;
    let mut notifies: Vec<(PolicyId, Effect)> = Vec::new();

    for policy in policies {
        if !filter_by_schedule(&policy.schedule, now) {
            continue;
        }

        match policy.effect {
            Effect::Allow | Effect::Block | Effect::TimeLimit { .. } => {
                if terminating.is_none() {
                    if let Effect::TimeLimit { limit_minutes } = policy.effect
                        && usage_minutes < limit_minutes
                    {
                        continue;
                    }
                    debug!(
                        policy_id = ?policy.id,
                        effect = ?policy.effect,
                        "evaluate: terminating match — breaking"
                    );
                    terminating = Some((policy.id, policy.effect));
                    break;
                }
            }
            Effect::Notify { limit_minutes } => {
                if usage_minutes >= limit_minutes {
                    debug!(
                        policy_id = ?policy.id,
                        limit_minutes,
                        usage_minutes,
                        "evaluate: Notify threshold exceeded"
                    );
                    notifies.push((policy.id, policy.effect));
                }
            }
        }
    }

    EvalResult {
        terminating,
        notifies,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::domain::{Effect, Policy, PolicyTarget};
    use wellbeing_core::{AppClass, PolicyId, TimeWindow};

    fn target_matches(
        target: &PolicyTarget,
        app_class: &AppClass,
        category_names: &[String],
    ) -> bool {
        match target {
            PolicyTarget::Any => true,
            PolicyTarget::App(t) => t == app_class,
            PolicyTarget::Category(cat_name) => category_names.contains(cat_name),
            PolicyTarget::Domain(_) => false, // DNS-level — excluded from app evaluation
        }
    }

    fn policy(id: i64, effect: Effect, priority: u64) -> Policy {
        Policy {
            id: PolicyId(id),
            name: format!("policy-{}", id),
            effect,
            target: PolicyTarget::Any,
            priority,
            schedule: vec![],
            time_limit_minutes: match effect {
                Effect::TimeLimit { limit_minutes } | Effect::Notify { limit_minutes } => {
                    limit_minutes
                }
                _ => 0,
            },
            user_id: 1000,
            created_by: 1000,
            created_at: "2026-01-01T00:00:00Z".into(),
            updated_at: "2026-01-01T00:00:00Z".into(),
        }
    }

    fn now() -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::parse_from_rfc3339("2026-07-17T10:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc)
    }

    #[test]
    fn test_empty_policies_unrestricted() {
        let result = evaluate(&[], 0, now());
        assert!(result.is_unrestricted());
    }

    #[test]
    fn test_block_terminates() {
        let policies = vec![policy(1, Effect::Block, 100)];
        let result = evaluate(&policies, 0, now());
        assert_eq!(result.terminating, Some((PolicyId(1), Effect::Block)));
        assert!(result.notifies.is_empty());
    }

    #[test]
    fn test_allow_terminates() {
        let policies = vec![policy(1, Effect::Allow, 100)];
        let result = evaluate(&policies, 0, now());
        assert_eq!(result.terminating, Some((PolicyId(1), Effect::Allow)));
    }

    #[test]
    fn test_time_limit_not_yet_exceeded() {
        let policies = vec![policy(1, Effect::TimeLimit { limit_minutes: 60 }, 100)];
        let result = evaluate(&policies, 30, now());
        // Not exceeded — no terminating match.
        assert!(result.terminating.is_none());
    }

    #[test]
    fn test_time_limit_exceeded() {
        let policies = vec![policy(1, Effect::TimeLimit { limit_minutes: 60 }, 100)];
        let result = evaluate(&policies, 61, now());
        assert_eq!(
            result.terminating,
            Some((PolicyId(1), Effect::TimeLimit { limit_minutes: 60 }))
        );
    }

    #[test]
    fn test_notify_non_terminating() {
        let policies = vec![
            policy(1, Effect::Notify { limit_minutes: 30 }, 100),
            policy(2, Effect::Block, 200),
        ];
        let result = evaluate(&policies, 40, now());
        // Notify matched and is in notifies, then Block terminates.
        assert_eq!(result.terminating, Some((PolicyId(2), Effect::Block)));
        assert_eq!(result.notifies.len(), 1);
        assert_eq!(result.notifies[0].0, PolicyId(1));
    }

    #[test]
    fn test_priority_order_first_match_wins() {
        // Lower priority = evaluated first.
        let policies = vec![
            policy(1, Effect::Block, 10),  // low priority, first
            policy(2, Effect::Allow, 100), // higher priority
        ];
        let result = evaluate(&policies, 0, now());
        assert_eq!(result.terminating, Some((PolicyId(1), Effect::Block)));
    }

    #[test]
    fn test_schedule_inactive_policy_skipped() {
        let mut p = policy(1, Effect::Block, 100);
        p.schedule = vec![TimeWindow {
            start_minute: 0,
            end_minute: 300, // 00:00–05:00 — now is 10:00, so inactive
            day_mask: 0x7F,
        }];
        let result = evaluate(&[p], 0, now());
        assert!(result.is_unrestricted());
    }

    #[test]
    fn test_notify_only_multiple() {
        let policies = vec![
            policy(1, Effect::Notify { limit_minutes: 30 }, 100),
            policy(2, Effect::Notify { limit_minutes: 60 }, 200),
        ];
        let result = evaluate(&policies, 45, now());
        assert!(result.terminating.is_none());
        assert_eq!(result.notifies.len(), 1); // Only policy 1 (limit 30) exceeded
        assert_eq!(result.notifies[0].0, PolicyId(1));
    }

    #[test]
    fn test_target_matches_app() {
        let app = AppClass::new("firefox").unwrap();
        let cat_a = "Social".to_string();
        let cat_b = "Entertainment".to_string();

        assert!(target_matches(
            &PolicyTarget::App(app.clone()),
            &app,
            std::slice::from_ref(&cat_a)
        ));
        assert!(!target_matches(
            &PolicyTarget::App(AppClass::new("code").unwrap()),
            &app,
            std::slice::from_ref(&cat_a)
        ));
        assert!(target_matches(
            &PolicyTarget::Category(cat_a.clone()),
            &app,
            &[cat_a.clone(), cat_b.clone()]
        ));
        assert!(!target_matches(
            &PolicyTarget::Category("Productivity".to_string()),
            &app,
            &[cat_a, cat_b]
        ));
        assert!(target_matches(&PolicyTarget::Any, &app, &[]));
    }
}
