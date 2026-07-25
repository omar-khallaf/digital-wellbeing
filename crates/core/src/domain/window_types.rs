use serde::{Deserialize, Serialize};
use zvariant::{Type, Value};

#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
pub enum BlockReason {
    AppTimeLimit = 0,
    CategoryTimeLimit = 1,
    AppBlock = 2,
    CategoryBlock = 3,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type, Value)]
pub struct BlockedAppEntry {
    pub app_id: String,
    pub policy_id: u64,
    pub reason: u32,
    pub blocked_since: u64,
}
