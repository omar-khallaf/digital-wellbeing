//! Policies domain types — pure data structures, no gpui dependency.

use wellbeing_core::{Category, PolicyData, PolicyInput, PolicyKind};

/// Pure-data ViewModel for the Policies screen.
#[derive(Debug, Clone)]
pub struct PoliciesViewModel {
    /// Every app id ever seen in the event log (for the dropdown selector).
    pub app_list: Vec<String>,
    /// Currently-edited policy target + form data, if any.
    pub selected_policy: Option<(PolicyTarget, PolicyConfigForm)>,
    /// All categories from the DB.
    pub categories: Vec<Category>,
    /// All policies for the current user (or all users if admin).
    pub policies: Vec<PolicyData>,
    /// Per-field validation error messages shown in the PolicyEditor.
    pub validation_errors: Vec<String>,
    /// Whether the current session uid owns admin privileges.
    pub is_admin: bool,
}

/// UI-level target for a policy — mirrors `Policy`'s two variants but carries
/// user-editable form data instead of a finalized domain value.
#[derive(Clone, Debug)]
pub enum PolicyTarget {
    /// Target an individual app by its `AppId` string.
    App(String),
    /// Target every app in a category by the category's row id.
    Category(i64),
}

/// Editable form fields for a single policy configuration.
#[derive(Clone, Debug)]
pub struct PolicyConfigForm {
    /// Policy kind discriminant string: `"Block"`, `"TimeLimit"`, or `"Notify"`.
    pub kind: String,
    /// Per-day time limit in minutes (only meaningful when kind == TimeLimit).
    pub time_limit_minutes: i64,
    /// One-shot extension grant in minutes.
    pub extra_minutes: i64,
    /// JSON-encoded schedule rules (see `TimeWindow` / `ScheduleRule`).
    pub schedule_json: String,
    /// Whether this policy is currently active / enforced.
    pub active: bool,
    /// Target app id (window class for Hyprland). Empty = no app target.
    pub app_id: String,
}

impl Default for PolicyConfigForm {
    fn default() -> Self {
        Self {
            kind: "Block".into(),
            time_limit_minutes: 60,
            extra_minutes: 0,
            schedule_json: "{}".into(),
            active: true,
            app_id: String::new(),
        }
    }
}

/// Build a `PolicyInput` from the editor form + target.
pub fn policy_input_from(
    target: PolicyTarget,
    form: &PolicyConfigForm,
    owner_id: u32,
) -> PolicyInput {
    let kind = match form.kind.as_str() {
        "TimeLimit" => PolicyKind::TimeLimit,
        "Notify" => PolicyKind::Notify,
        _ => PolicyKind::Block,
    };
    let (app_id, category_id) = match target {
        PolicyTarget::App(_) => (form.app_id.clone(), 0),
        PolicyTarget::Category(id) => (String::new(), id),
    };
    PolicyInput {
        name: format!("policy-{}", app_cat_label(category_id, &app_id)),
        action: kind,
        app_id: app_id.clone(),
        category_id,
        time_limit_minutes: form.time_limit_minutes,
        extra_minutes: form.extra_minutes,
        notification_repeat_interval_minutes: 0,
        schedule_json: form.schedule_json.clone(),
        active: form.active,
        owner_id,
    }
}

fn app_cat_label(cat_id: i64, app_id: &str) -> String {
    if cat_id > 0 {
        format!("cat-{}", cat_id)
    } else {
        app_id.to_string()
    }
}
