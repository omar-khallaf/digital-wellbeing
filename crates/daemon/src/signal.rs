use wellbeing_core::{AppClass, BlockReason, DomainPattern, Uid};

/// Signals emitted by actors and forwarded to D-Bus by main.rs.
#[derive(Debug, Clone)]
pub enum DaemonSignal {
    AppBlocked {
        uid: Uid,
        app_class: AppClass,
        blocked: bool,
        reason: BlockReason,
    },
    DomainBlocked {
        uid: Uid,
        domain: DomainPattern,
        blocked: bool,
        reason: BlockReason,
    },
    PolicyChanged {
        uid: Uid,
    },
    UsageUpdated {
        uid: Uid,
    },
}
