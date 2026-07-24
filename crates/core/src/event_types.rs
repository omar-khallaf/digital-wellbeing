//! Shared event type constants for the Digital Wellbeing system.
//!
//! Single source of truth for the eight event types that cover every
//! focus switch and state change. Used by both the daemon (persistence
//! layer, enforcement) and the GUI (day timeline builder, hourly
//! aggregation).
//!
//! Daemon stores event_type as i32 (SQLite INTEGER), while GUI
//! deserializes from D-Bus as u8. Both are represented here as i32;
//! consumers cast as needed.

/// App window gained focus — opens a focus interval.
pub const EVENT_WINDOW_FOCUSED: i32 = 0;

/// No window is focused (desktop) — closes a focus interval.
pub const EVENT_UNFOCUSED: i32 = 1;

/// User became idle — focus interval is paused.
pub const EVENT_IDLE: i32 = 2;

/// User resumed from idle — focus interval resumes.
pub const EVENT_RESUMED: i32 = 3;

/// System entered sleep — closes a focus interval.
pub const EVENT_SLEPT: i32 = 4;

/// System shut down — closes a focus interval.
pub const EVENT_SHUT_DOWN: i32 = 5;

/// Session locked — closes a focus interval.
pub const EVENT_LOCKED: i32 = 6;

/// User logged out — closes a focus interval.
pub const EVENT_LOGGED_OUT: i32 = 7;

pub const CLOSE_EVENT_TYPES: &[i32] = &[
    EVENT_UNFOCUSED,
    EVENT_SLEPT,
    EVENT_SHUT_DOWN,
    EVENT_LOCKED,
    EVENT_LOGGED_OUT,
];

/// Accepts any integer type via `Into<i32>` so both the daemon (i32
/// from SQLite) and the GUI (u8 from D-Bus) can use it without
/// explicit casting at each call site.
pub fn is_close_event_type(event_type: impl Into<i32>) -> bool {
    CLOSE_EVENT_TYPES.contains(&event_type.into())
}
