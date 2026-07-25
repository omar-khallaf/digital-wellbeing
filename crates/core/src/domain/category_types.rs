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

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct AppCategoryRow {
    pub app_id: String,
    pub user_id: u32,
    pub category_id: i64,
    pub display_name: String,
    pub icon_path: String,
    pub ignore: bool,
}
