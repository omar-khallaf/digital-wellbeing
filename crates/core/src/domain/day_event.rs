use serde::{Deserialize, Serialize};
use zvariant::Type;

/// A single row from the `events` table, exposed over D-Bus.
///
/// Returned by `get_day_events` — raw event data for a user+day.
///
/// D-Bus structs have no optional fields, so missing `app_id`/`title` are
/// represented as empty strings (matching the codebase sentinel convention).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct DayEventRow {
    pub id: u64,
    pub event_type: u8,
    pub timestamp: i64,
    pub app_id: String,
    pub title: String,
    pub user_id: u64,
}
