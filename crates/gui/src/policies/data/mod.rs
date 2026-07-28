//! Policies data layer — ViewModel builder, repository, and background flow.

mod flow;
mod repo;

pub use flow::spawn_policies_flow;
pub use repo::{PoliciesData, PoliciesRepo};

use super::domain::PoliciesViewModel;

// ---------------------------------------------------------------------------
// ViewModel mutation method (Compose-like StateFlow pattern)
// ---------------------------------------------------------------------------

impl PoliciesViewModel {
    /// Recompute ALL derived fields from the raw data in `self.data`.
    ///
    /// Call after a full fetch (policy_mutated signal / daemon reconnect /
    /// manual refresh).  Preserves `selected_policy` and `is_admin` — those
    /// are UI editing state, not derived from raw data.
    pub fn recompute_derived(&mut self) {
        let Some(ref data) = self.data else {
            self.app_list.clear();
            self.categories.clear();
            self.policies.clear();
            self.validation_errors.clear();
            return;
        };

        self.app_list = data
            .app_list
            .iter()
            .map(|ac| ac.app_class.clone())
            .collect();
        self.categories = data.categories.clone();
        self.policies = data.policies.clone();
        self.validation_errors.clear();
        // selected_policy: preserved (UI state set by App.policy_edit).
    }
}
