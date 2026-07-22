//! Policies ViewModel builder — pure function, no gpui.

use wellbeing_core::{Category, PolicyData};

use super::domain::PoliciesViewModel;

/// Build a `PoliciesViewModel` from the raw data sources the D-Bus client /
/// cache provides.
pub fn build_policies_viewmodel(
    policies: &[PolicyData],
    categories: &[Category],
    app_ids: &[String],
    is_admin: bool,
) -> PoliciesViewModel {
    PoliciesViewModel {
        app_list: app_ids.to_vec(),
        selected_policy: None,
        categories: categories.to_vec(),
        policies: policies.to_vec(),
        validation_errors: Vec::new(),
        is_admin,
    }
}
