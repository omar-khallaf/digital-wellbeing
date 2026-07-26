use wellbeing_core::{AppClass, BlockReason, Uid};

/// Signals emitted by actors and forwarded to D-Bus by main.rs.
#[derive(Debug, Clone)]
pub enum DaemonSignal {
    BlockedAppsChanged {
        uid: Uid,
        app_class: AppClass,
        blocked: bool,
        reason: BlockReason,
    },
    /// Daily usage data changed for a user — consumers should re-query.
    DailyUsageChanged { uid: Uid },
}
