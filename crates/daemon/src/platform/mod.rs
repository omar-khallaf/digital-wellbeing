//! Platform event types — the sole input contract for the daemon.
//!
//! All events carry a uid (the plugin instance that produced them).  There
//! are no system-wide events.  The plugin is the sole source of truth for
//! current window focus state.
pub mod linux;

use futures::Stream;
use wellbeing_core::{AppId, Pid, Uid, WindowTitle};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerEventKind {
    Suspend,
    Hibernate,
    Shutdown,
}

#[derive(Debug, Clone)]
pub enum PlatformEvent {
    Focus {
        app_id: AppId,
        title: WindowTitle,
        pid: Pid,
        uid: Uid,
    },
    Unfocus {
        uid: Uid,
    },
    Block {
        app_id: AppId,
        title: WindowTitle,
        uid: Uid,
    },
    Idle {
        uid: Uid,
    },
    Resume {
        uid: Uid,
    },
    LogOut {
        uid: Uid,
    },
    Locked {
        uid: Uid,
    },
    PowerEvent {
        kind: PowerEventKind,
        uid: Uid,
    },
}

pub trait Platform: Send + Sync + 'static {
    type EventStream: Stream<Item = PlatformEvent> + Send + 'static;

    fn notify(
        &self,
        title: &str,
        body: &str,
    ) -> impl std::future::Future<Output = anyhow::Result<()>> + Send;
}

impl PlatformEvent {
    /// DB `event_type` discriminator for this variant.
    pub fn event_type(&self) -> i32 {
        use wellbeing_core::event_types::*;
        match self {
            PlatformEvent::Focus { .. } => EVENT_WINDOW_FOCUSED,
            PlatformEvent::Block { .. } => EVENT_WINDOW_BLOCKED,
            PlatformEvent::Unfocus { .. } => EVENT_UNFOCUSED,
            PlatformEvent::Idle { .. } => EVENT_IDLE,
            PlatformEvent::Resume { .. } => EVENT_RESUMED,
            PlatformEvent::LogOut { .. } => EVENT_LOGGED_OUT,
            PlatformEvent::Locked { .. } => EVENT_LOCKED,
            PlatformEvent::PowerEvent {
                kind: PowerEventKind::Suspend | PowerEventKind::Hibernate,
                ..
            } => EVENT_SLEPT,
            PlatformEvent::PowerEvent {
                kind: PowerEventKind::Shutdown,
                ..
            } => EVENT_SHUT_DOWN,
        }
    }

    /// Whether this event closes a currently-open focus interval.
    pub fn is_close_event(&self) -> bool {
        wellbeing_core::event_types::CLOSE_EVENT_TYPES.contains(&self.event_type())
    }

    /// Uid associated with this event — all variants carry one.
    pub fn uid(&self) -> Uid {
        match self {
            PlatformEvent::Focus { uid, .. }
            | PlatformEvent::Unfocus { uid }
            | PlatformEvent::Block { uid, .. }
            | PlatformEvent::Idle { uid }
            | PlatformEvent::Resume { uid }
            | PlatformEvent::LogOut { uid }
            | PlatformEvent::Locked { uid }
            | PlatformEvent::PowerEvent { uid, .. } => *uid,
        }
    }

    /// App id associated with this event, if any.
    pub fn app_id(&self) -> Option<&AppId> {
        match self {
            PlatformEvent::Focus { app_id, .. } | PlatformEvent::Block { app_id, .. } => {
                Some(app_id)
            }
            _ => None,
        }
    }

    /// Window title associated with this event, if any.
    pub fn title(&self) -> Option<&WindowTitle> {
        match self {
            PlatformEvent::Focus { title, .. } | PlatformEvent::Block { title, .. } => Some(title),
            _ => None,
        }
    }
}
