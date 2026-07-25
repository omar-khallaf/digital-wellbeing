//! Data access helpers for D-Bus interface methods.
//!
//! Sub-modules organized by domain concern, each < 250 LOC.

mod category_data;
mod events;
mod policy_data;
mod usage_data;

pub(crate) use category_data::{get_app_categories, list_categories, set_app_category};
pub(crate) use events::get_day_events;
pub(crate) use policy_data::{
    create_policy, delete_policy, get_policy_owner, list_policies, update_policy,
};
pub(crate) use usage_data::{
    get_daily_usage, get_daily_usage_by_title, get_usage_range, get_usage_range_by_title,
};
