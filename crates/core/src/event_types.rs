//! Event types for the Digital Wellbeing system.
//!
//! The [`EventType`] enum is the single source of truth for all nine
//! focus-switch / state-change discriminants.  Domain code uses the enum;
//! [`From`]/[`TryFrom`] conversions bridge to the integer representations
//! needed at the D-Bus and database boundaries.
//!
//! # Wire format constraints
//!
//! - Daemon stores `event_type` as `i32` (SQLite INTEGER).
//! - GUI deserializes from D-Bus as `u8`.
//! - The `Event` D-Bus signal carries a **separate** 4-field struct
//!   (`(ussu)`) whose first field is a `u32` **tag** (see
//!   [`dbus_constants`]).  That tag is *not* `EventType` — it is a
//!   higher-level discriminator that covers power-events and is handled by
//!   the platform layer.

use serde_repr::{Deserialize_repr, Serialize_repr};
use zvariant::Type;

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize_repr, Deserialize_repr, Type)]
pub enum EventType {
    Focus = 0,
    Unfocus = 1,
    Idle = 2,
    Resume = 3,
    Suspend = 4,
    ShutDown = 5,
    Locked = 6,
    LoggedOut = 7,
    Block = 8,
}

// ── Boundary conversions ─────────────────────────────────────────────────────
// These are used at the DB (i32) and D-Bus (u8) boundaries only.
// Domain code should never call these unless it is crossing a boundary.

impl From<EventType> for i32 {
    fn from(et: EventType) -> Self {
        et as i32
    }
}

impl TryFrom<i32> for EventType {
    type Error = &'static str;

    fn try_from(v: i32) -> Result<Self, Self::Error> {
        match v {
            0 => Ok(EventType::Focus),
            1 => Ok(EventType::Unfocus),
            2 => Ok(EventType::Idle),
            3 => Ok(EventType::Resume),
            4 => Ok(EventType::Suspend),
            5 => Ok(EventType::ShutDown),
            6 => Ok(EventType::Locked),
            7 => Ok(EventType::LoggedOut),
            8 => Ok(EventType::Block),
            _ => Err("unknown event type discriminant"),
        }
    }
}

impl From<EventType> for u8 {
    fn from(et: EventType) -> Self {
        et as u8
    }
}

impl TryFrom<u8> for EventType {
    type Error = &'static str;

    fn try_from(v: u8) -> Result<Self, Self::Error> {
        Self::try_from(v as i32)
    }
}

// ── Queries on EventType ─────────────────────────────────────────────────────

impl EventType {
    /// Event types that *close* a focus interval.
    pub const CLOSE: [EventType; 6] = [
        EventType::Unfocus,
        EventType::Suspend,
        EventType::ShutDown,
        EventType::Locked,
        EventType::LoggedOut,
        EventType::Block,
    ];

    /// Event types that are **ignored** by interval measurement (Idle/Resumed
    /// do not open, close, or split an interval).
    pub const IGNORED_BY_MEASUREMENT: [EventType; 2] = [EventType::Idle, EventType::Resume];

    /// Whether this event type closes a focus interval.
    pub fn is_close(self) -> bool {
        Self::CLOSE.contains(&self)
    }

    /// Whether this event type is ignored by interval measurement.
    pub fn is_ignored_by_measurement(self) -> bool {
        Self::IGNORED_BY_MEASUREMENT.contains(&self)
    }
}
