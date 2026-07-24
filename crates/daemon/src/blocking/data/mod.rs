//! Data access layer for blocking/enforcement module.

mod persistence;
pub(crate) use persistence::BlockingRepo;
pub use persistence::EventRow;
pub use wellbeing_core::event_types::{
    CLOSE_EVENT_TYPES, EVENT_IDLE, EVENT_RESUMED, EVENT_WINDOW_FOCUSED,
};
