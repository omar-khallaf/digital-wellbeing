//! Policies domain types — pure data structures, no gpui dependency.

use wellbeing_core::{
    AppClass, Category, DomainPattern, Effect, PolicyData, PolicyInput, TargetType, TimeWindow, Uid,
};

use super::data::PoliciesData;

/// Pure-data ViewModel for the Policies screen.
///
/// Acts like a Compose ViewModel with `StateFlow` — raw data persists in
/// `self.data` and derived fields are recomputed via `recompute_derived()`.
#[derive(Debug, Clone)]
pub struct PoliciesViewModel {
    /// Raw data bundle (like `MutableStateFlow<PoliciesData?>`).
    /// `None` before the first successful fetch.
    pub data: Option<PoliciesData>,
    /// Every app id ever seen in the event log (for the dropdown selector).
    pub app_list: Vec<AppClass>,
    /// Currently-edited policy target + form data, if any.
    pub selected_policy: Option<(PolicyTarget, PolicyConfigForm)>,
    pub categories: Vec<Category>,
    /// All policies for the current user (or all users if admin).
    pub policies: Vec<PolicyData>,
    /// Per-field validation error messages shown in the PolicyEditor.
    pub validation_errors: Vec<String>,
    pub is_admin: bool,
}

impl Default for PoliciesViewModel {
    fn default() -> Self {
        Self {
            data: None,
            app_list: Vec::new(),
            selected_policy: None,
            categories: Vec::new(),
            policies: Vec::new(),
            validation_errors: Vec::new(),
            is_admin: false,
        }
    }
}

/// UI-level target for a policy.
#[derive(Clone, Debug)]
pub enum PolicyTarget {
    /// Target an individual app by its `AppClass`.
    App(AppClass),
    /// Target every app in a category by the category's name.
    Category(String),
    /// Target a domain pattern.
    Domain(DomainPattern),
    /// Wildcard — matches everything.
    Any,
}

/// Editable form fields for a single policy configuration.
#[derive(Clone, Debug)]
pub struct PolicyConfigForm {
    /// Policy effect string: `"Allow"`, `"Block"`, `"TimeLimit"`, or `"Notify"`.
    pub kind: String,
    /// Per-day time limit in minutes (only meaningful when kind == TimeLimit/Notify).
    pub time_limit_minutes: i64,
    /// JSON-encoded schedule rules (Vec<TimeWindow>).
    pub schedule_json: String,
    /// Parsed schedule rules from `schedule_json` — UI state, not serialized.
    pub schedules: Vec<TimeWindow>,
    pub app_class: AppClass,
    /// Target category name — valid when target_type is Category.
    pub category_name: String,
    /// Priority (lower = evaluated first). Default 100.
    pub priority: i64,
    /// Working day-mask for the "add new window" controls. 7-bit bitmask (0x7F = all days).
    pub schedule_new_day_mask: u8,
}

impl Default for PolicyConfigForm {
    fn default() -> Self {
        Self {
            kind: "Block".into(),
            time_limit_minutes: 60,
            schedule_json: "[]".into(),
            schedules: vec![],
            app_class: AppClass::new("_").expect("static sentinel is non-empty"),
            category_name: String::new(),
            priority: 100,
            schedule_new_day_mask: 0x7F,
        }
    }
}

pub fn policy_input_from(
    target: PolicyTarget,
    form: &PolicyConfigForm,
    user_id: u32,
) -> PolicyInput {
    let effect = match form.kind.as_str() {
        "Allow" => Effect::Allow,
        "TimeLimit" => Effect::TimeLimit,
        "Notify" => Effect::Notify,
        _ => Effect::Block,
    };
    let placeholder_app_class =
        || AppClass::new("_").expect("static sentinel '_' is a valid non-empty AppClass");
    let placeholder_domain =
        || DomainPattern::new("_").expect("static sentinel '_' is a valid non-empty DomainPattern");
    let (target_type, app_class, category_name, domain_pattern) = match target {
        PolicyTarget::App(_) => (
            TargetType::App,
            form.app_class.clone(),
            String::new(),
            placeholder_domain(),
        ),
        PolicyTarget::Category(_) => (
            TargetType::Category,
            placeholder_app_class(),
            form.category_name.clone(),
            placeholder_domain(),
        ),
        PolicyTarget::Domain(d) => (
            TargetType::Domain,
            placeholder_app_class(),
            String::new(),
            d,
        ),
        PolicyTarget::Any => (
            TargetType::Any,
            placeholder_app_class(),
            String::new(),
            placeholder_domain(),
        ),
    };
    PolicyInput {
        name: match target_type {
            TargetType::App => format!("policy-{}", app_class),
            TargetType::Category => format!("policy-cat-{}", category_name),
            TargetType::Domain => format!("policy-domain-{}", domain_pattern),
            TargetType::Any => format!("policy-any-{}", form.kind),
        },
        effect,
        target_type,
        app_class,
        category_name,
        domain_pattern,
        priority: form.priority,
        time_limit_minutes: form.time_limit_minutes,
        schedule_json: form.schedule_json.clone(),
        user_id: Uid(user_id),
    }
}
