//! Policy evaluation logic — decides whether to block, notify, or allow.

use wellbeing_core::BlockReason;

use crate::policy::domain::{PolicyConfig, PolicyVerdict};

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
            ..
        } => {
            let effective_limit_minutes = *time_limit_minutes;
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
pub fn evaluate(policies: &[PolicyConfig], elapsed_usage: i64) -> PolicyVerdict {
    let mut first_block: Option<PolicyVerdict> = None;
    let mut first_notify: Option<PolicyVerdict> = None;

    for policy in policies {
        if !policy.active() {
            continue;
        }
        if let Some(block) = eval_block_policy(policy, first_block.is_some())
            .or_else(|| eval_timelimit_policy(policy, elapsed_usage, first_block.is_some()))
        {
            first_block = Some(block);
        } else if let Some(notify) =
            eval_notify_policy(policy, elapsed_usage, first_notify.is_some())
        {
            first_notify = Some(notify);
        }
    }
    first_block.or(first_notify).unwrap_or(PolicyVerdict::Ok)
}
