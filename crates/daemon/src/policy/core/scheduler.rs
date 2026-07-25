//! Schedule-based policy filtering.

use chrono::{DateTime, Utc};

use crate::policy::domain::{Policy, PolicyConfig};

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
