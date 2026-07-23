//! Domain types for the blocking/enforcement feature.

use std::time::SystemTime;

use tokio::sync::oneshot;
use wellbeing_core::{AppId, PolicyId, Uid};

/// Internal events for the blocking actor.
pub enum InternalEvent {
    /// Flush the event buffer. The optional oneshot sender is signaled
    /// after the flush completes, allowing callers (e.g. shutdown) to
    /// await completion instead of guessing with a sleep.
    Flush(Option<oneshot::Sender<()>>),
}

/// Top-level blocking state machine.
#[derive(Debug, Clone)]
pub enum BlockingState {
    Idle,
    OverlayShown {
        app_id: AppId,
        policy_id: PolicyId,
        blocked_since: SystemTime,
        uid: Uid,
    },
}
