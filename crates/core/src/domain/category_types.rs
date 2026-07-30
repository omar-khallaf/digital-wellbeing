use crate::valuetypes::*;
use serde::{Deserialize, Serialize};
use zvariant::Type;

/// App-to-category assignment row.
///
/// `category` holds the [`Category`] enum variant directly.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct AppCategoryRow {
    pub app_class: AppClass,
    pub user_id: Uid,
    pub category: Category,
    pub display_name: String,
    pub icon_path: String,
    pub ignore: bool,
}
