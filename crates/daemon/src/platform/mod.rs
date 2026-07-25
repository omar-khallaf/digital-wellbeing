use futures::Stream;
use wellbeing_core::{AppId, Pid, Uid, WindowTitle};

#[derive(Debug, Clone)]
pub enum PlatformEvent {
    WindowFocused {
        app_id: AppId,
        title: WindowTitle,
        pid: Pid,
        uid: Uid,
    },
    WindowBlocked {
        app_id: AppId,
        title: WindowTitle,
        uid: Uid,
    },
    Unfocused,
    IdleActivity,
    ResumedActivity,
    ResumedSystem,
    Slept,
    ShutDown,
    Locked,
    LoggedOut,
}

pub trait Platform: Send + Sync + 'static {
    type EventStream: Stream<Item = PlatformEvent> + Send + 'static;

    fn notify(
        &self,
        title: &str,
        body: &str,
    ) -> impl std::future::Future<Output = anyhow::Result<()>> + Send;
}

pub mod linux;
