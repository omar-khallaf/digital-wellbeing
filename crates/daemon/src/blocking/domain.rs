//! Domain types for the blocking/enforcement feature.

use std::time::{Duration, SystemTime};

use wellbeing_core::{AppId, PolicyId, Uid};

/// Internal events for timer-driven enforcement actions.
pub(crate) enum InternalEvent {
    LimitReached(AppId),
    NotifyTick(AppId),
}

/// State for a running notification repeat timer.
#[derive(Debug)]
pub struct NotifyTimerState {
    pub policy_id: PolicyId,
    pub repeat_interval: Duration,
    pub(crate) handle: tokio::task::JoinHandle<()>,
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
