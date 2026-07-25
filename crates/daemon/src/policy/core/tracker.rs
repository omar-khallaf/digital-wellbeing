//! Tracked state computation for apps under policy.

use crate::policy::domain::{PolicyConfig, TimeLimitedApp, TimeTrackedApp, TrackedApp};

/// Compute the tracked state for an app given usage data and policy config.
pub fn app_state(usage: i64, policy: &PolicyConfig) -> TrackedApp {
    match policy {
        PolicyConfig::Block { .. } => {
            unreachable!("Block policy has no tracked state")
        }
        PolicyConfig::TimeLimit {
            time_limit_minutes, ..
        } => TrackedApp::TimeLimited(TimeLimitedApp {
            used: usage,
            limit: *time_limit_minutes,
        }),
        PolicyConfig::Notify {
            time_limit_minutes, ..
        } => TrackedApp::TimeTracked(TimeTrackedApp {
            used: usage,
            limit: *time_limit_minutes,
        }),
    }
}
