//! Policies data layer — ViewModel builder, repository, and background flow.

mod flow;
mod repo;

pub use flow::spawn_policies_flow;
pub use repo::PoliciesRepo;

use wellbeing_core::{AppClass, Category, PolicyData};

use super::domain::PoliciesViewModel;

/// Build a `PoliciesViewModel` from the raw data sources the repository
/// provides.
pub fn build_policies_viewmodel(
    policies: &[PolicyData],
    categories: &[Category],
    app_classs: &[AppClass],
    is_admin: bool,
) -> PoliciesViewModel {
    PoliciesViewModel {
        app_list: app_classs.to_vec(),
        selected_policy: None,
        categories: categories.to_vec(),
        policies: policies.to_vec(),
        validation_errors: Vec::new(),
        is_admin,
    }
}
