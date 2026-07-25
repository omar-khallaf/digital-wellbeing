//! A platform event with the handling timestamp attached.

use crate::platform::PlatformEvent;
use chrono::{DateTime, Utc};

/// A platform event with the handling timestamp attached.
#[derive(Debug, Clone)]
pub(crate) struct TimedEvent {
    pub event: PlatformEvent,
    pub timestamp: DateTime<Utc>,
}
