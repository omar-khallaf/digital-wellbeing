use futures::Stream;
use wellbeing_core::{AppId, Pid, Uid, WindowTitle};

#[derive(Debug, Clone)]
pub enum PlatformEvent {
    WindowFocused {
        app_id: AppId,
        title: WindowTitle,
        pid: Pid,
        uid: Uid,
        overlay_shown: bool,
    },
    Unfocused,
    IdleActivity,
    ResumedActivity,
    ResumedSystem,
    Slept,
    ShutDown,
    Locked,
    LoggedOut,
    UserAction {
        app_id: AppId,
        action: u32,
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

pub mod linux;
