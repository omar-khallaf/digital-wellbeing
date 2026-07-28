//! Platform event types — the sole input contract for the daemon.
//!
//! All events carry a uid (the plugin instance that produced them).  There
//! are no system-wide events.  The plugin is the sole source of truth for
//! current window focus state.
pub mod linux;

use futures::Stream;
use wellbeing_core::{AppClass, EventType, Uid, WindowTitle};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerEventKind {
    Suspend,
    Hibernate,
    Shutdown,
}

#[derive(Debug, Clone)]
pub enum PlatformEvent {
    Focus {
        app_class: AppClass,
        title: WindowTitle,
        uid: Uid,
    },
    Unfocus {
        uid: Uid,
    },
    Block {
        app_class: AppClass,
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
    pub fn event_type(&self) -> EventType {
        match self {
            PlatformEvent::Focus { .. } => EventType::Focus,
            PlatformEvent::Block { .. } => EventType::Block,
            PlatformEvent::Unfocus { .. } => EventType::Unfocus,
            PlatformEvent::Idle { .. } => EventType::Idle,
            PlatformEvent::Resume { .. } => EventType::Resume,
            PlatformEvent::LogOut { .. } => EventType::LoggedOut,
            PlatformEvent::Locked { .. } => EventType::Locked,
            PlatformEvent::PowerEvent {
                kind: PowerEventKind::Suspend | PowerEventKind::Hibernate,
                ..
            } => EventType::Suspend,
            PlatformEvent::PowerEvent {
                kind: PowerEventKind::Shutdown,
                ..
            } => EventType::ShutDown,
        }
    }

    /// Whether this event closes a currently-open focus interval.
    pub fn is_close_event(&self) -> bool {
        self.event_type().is_close()
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
    pub fn app_class(&self) -> Option<&AppClass> {
        match self {
            PlatformEvent::Focus { app_class, .. } | PlatformEvent::Block { app_class, .. } => {
                Some(app_class)
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
