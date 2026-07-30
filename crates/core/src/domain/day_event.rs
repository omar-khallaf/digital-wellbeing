use serde::{Deserialize, Serialize};
use zvariant::Type;

use crate::event_types::EventType;
use crate::valuetypes::{AppClass, WindowTitle};

/// A single row from the `events` table, exposed over D-Bus.
///
/// Returned by `get_day_events` — raw event data for a user+day.
///
/// Fields use validated domain types where the D-Bus signature is preserved
/// (`EventType` has the same `y` signature as `u8`; `AppClass` has the same `s`
/// signature as `String`).  `user_id` stays as `u64` because `Uid` has
/// signature `u` (u32) — changing it would break the D-Bus wire contract.
///
/// Missing `app_class`/`title` are represented as empty strings (matching the
/// codebase sentinel convention for D-Bus structs with no optional fields).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct DayEventRow {
    pub id: u64,
    pub event_type: EventType,
    pub timestamp: i64,
    pub app_class: AppClass,
    pub title: WindowTitle,
    pub user_id: u64,
}
