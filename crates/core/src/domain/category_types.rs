use crate::valuetypes::*;
use serde::{Deserialize, Serialize};
use zvariant::Type;

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct Category {
    pub id: CategoryId,
    pub name: String,
    pub color: String,
    pub icon: String,
}

/// App-to-category assignment row.
///
/// Fields use validated domain types where the D-Bus signature is preserved
/// (`AppClass` has the same `s` signature as `String`; `Uid` has the same `u`
/// signature as `u32`).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct AppCategoryRow {
    pub app_class: AppClass,
    pub user_id: Uid,
    pub category_id: CategoryId,
    pub display_name: String,
    pub icon_path: String,
    pub ignore: bool,
}
