use wellbeing_core::AppId;

/// Signals emitted by actors and forwarded to D-Bus by main.rs.
#[derive(Debug, Clone)]
pub enum DaemonSignal {
    /// Block state changed for an app (shown / hidden).
    BlockStateChanged {
        uid: u32,
        app_id: AppId,
        blocked: bool,
        reason: u32,
    },
    /// Daily usage data changed for a user — consumers should re-query.
    DailyUsageChanged { uid: u32 },
}
