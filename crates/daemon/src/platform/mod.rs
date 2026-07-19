use std::time::SystemTime;

use futures::Stream;
use wellbeing_core::{AppId, BlockReason, OverlayAction, Pid, PolicyId, Uid, WindowTitle};

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
    Idle,
    Resumed,
    Slept,
    ShutDown,
    Locked,
    LoggedOut,
    UserAction {
        app_id: AppId,
        action: u32,
        policy_id: PolicyId,
    },
}

#[derive(Debug, Clone)]
pub struct OverlayConfig {
    pub app_id: AppId,
    pub policy_id: PolicyId,
    pub reason: BlockReason,
    pub blocked_since: SystemTime,
    pub available_actions: Vec<OverlayAction>,
}

#[allow(async_fn_in_trait)]
pub trait Platform: Send + Sync + 'static {
    type EventStream: Stream<Item = PlatformEvent> + Send + 'static;

    async fn show_overlay(&self, config: OverlayConfig, uid: Uid) -> anyhow::Result<()>;
    async fn hide_overlay(&self, app_id: &AppId, uid: Uid) -> anyhow::Result<()>;
    async fn notify(&self, title: &str, body: &str) -> anyhow::Result<()>;
}

pub mod linux;
