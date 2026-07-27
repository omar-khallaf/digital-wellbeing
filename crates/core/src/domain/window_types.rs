use serde::{Deserialize, Serialize};
use serde_repr::{Deserialize_repr, Serialize_repr};
use zvariant::{Type, Value};

use crate::valuetypes::*;

/// Reason why an app was blocked.
///
/// Serializes as [`u32`] on the D-Bus wire (`#[repr(u32)]`).  Domain code
/// matches on this enum; the integer conversion lives only at the D-Bus
/// boundary via [`From`]/[`TryFrom`].
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize_repr, Deserialize_repr, Type, Value)]
pub enum BlockReason {
    AppTimeLimit = 0,
    CategoryTimeLimit = 1,
    AppBlock = 2,
    CategoryBlock = 3,
}

impl From<BlockReason> for u32 {
    fn from(r: BlockReason) -> Self {
        r as u32
    }
}

impl TryFrom<u32> for BlockReason {
    type Error = &'static str;

    fn try_from(v: u32) -> Result<Self, Self::Error> {
        match v {
            0 => Ok(BlockReason::AppTimeLimit),
            1 => Ok(BlockReason::CategoryTimeLimit),
            2 => Ok(BlockReason::AppBlock),
            3 => Ok(BlockReason::CategoryBlock),
            _ => Err("unknown BlockReason discriminant"),
        }
    }
}

/// An entry in the blocked-apps map that is exposed over D-Bus.
///
/// All fields use validated domain types; the integer wire format is
/// handled by serde / zvariant at the D-Bus boundary.
#[derive(Debug, Clone, Serialize, Deserialize, Type, Value)]
pub struct BlockedAppEntry {
    pub app_class: AppClass,
    pub policy_id: PolicyId,
    pub reason: BlockReason,
    pub blocked_since: u64,
}

/// Payload for the `BlockedAppsChanged` D-Bus signal.
///
/// Emitted when a block is added or removed for a user.  The D-Bus wire
/// signature is `(usbu)` — matching the field order below.
///
/// `blocked` is `true` when the app is now blocked and `false` when unblocked.
#[derive(Debug, Clone, Serialize, Deserialize, Type, Value)]
pub struct BlockedAppsChangedSignal {
    pub uid: Uid,
    pub app_class: AppClass,
    pub blocked: bool,
    pub reason: BlockReason,
}
