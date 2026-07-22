//! Tracker actor — tracks active window focus state per user.

use std::collections::HashMap;

use wellbeing_core::{Clock, Uid};

use crate::platform::PlatformEvent;

use super::domain::{FocusState, ReactiveNotifier};

/// Tracks active window focus events and emits signals on state changes.
pub struct TrackerActor<C: Clock> {
    focus_state: HashMap<Uid, FocusState>,
    notifier: ReactiveNotifier,
    signal_tx: tokio::sync::mpsc::UnboundedSender<crate::signal::DaemonSignal>,
    clock: C,
}

impl<C: Clock> TrackerActor<C> {
    pub fn new(
        notifier: ReactiveNotifier,
        clock: C,
        signal_tx: tokio::sync::mpsc::UnboundedSender<crate::signal::DaemonSignal>,
    ) -> Self {
        Self {
            focus_state: HashMap::new(),
            notifier,
            signal_tx,
            clock,
        }
    }

    pub async fn handle_event(&mut self, event: PlatformEvent) {
        let now = self.clock.now();

        match event {
            PlatformEvent::WindowFocused { app_id, uid, .. } => {
                self.focus_state.insert(uid, FocusState::new(app_id, now));
                let _ = self
                    .signal_tx
                    .send(crate::signal::DaemonSignal::DailyUsageChanged { uid: uid.0 });
            }

            PlatformEvent::Unfocused | PlatformEvent::ShutDown | PlatformEvent::LoggedOut => {
                let uids: Vec<Uid> = self.focus_state.keys().copied().collect();
                for uid in &uids {
                    self.focus_state.remove(uid);
                    let _ = self
                        .signal_tx
                        .send(crate::signal::DaemonSignal::DailyUsageChanged { uid: uid.0 });
                }
            }

            PlatformEvent::Idle | PlatformEvent::Slept | PlatformEvent::Locked => {
                let uids: Vec<Uid> = self.focus_state.keys().copied().collect();
                for uid in &uids {
                    if let Some(fs) = self.focus_state.get_mut(uid) {
                        fs.pause(now);
                    }
                    let _ = self
                        .signal_tx
                        .send(crate::signal::DaemonSignal::DailyUsageChanged { uid: uid.0 });
                }
            }

            PlatformEvent::Resumed => {
                let uids: Vec<Uid> = self.focus_state.keys().copied().collect();
                for uid in &uids {
                    if let Some(fs) = self.focus_state.get_mut(uid) {
                        fs.resume(now);
                    }
                    let _ = self
                        .signal_tx
                        .send(crate::signal::DaemonSignal::DailyUsageChanged { uid: uid.0 });
                }
            }

            PlatformEvent::UserAction { .. } => {}
        }

        self.notifier.notify();
    }

    pub async fn run(mut self, mut rx: tokio::sync::mpsc::Receiver<PlatformEvent>) {
        while let Some(event) = rx.recv().await {
            self.handle_event(event).await;
        }
        tracing::info!("tracker: event loop ended (channel closed)");
    }

    pub fn focus_state(&self) -> &HashMap<Uid, FocusState> {
        &self.focus_state
    }
}
