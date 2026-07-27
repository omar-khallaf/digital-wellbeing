//! ReportsFlow — independent background task with signal-triggered refresh,
//! presence-aware reconnection, and timeout-guarded D-Bus fetches.
//!
//! Relies on the daemon's `DailyUsageChanged` signal (emitted every minute) to
//! drive re-fetches — no polling ticker needed.

use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use tokio::sync::RwLock;
use tokio::sync::broadcast;
use tokio::sync::mpsc::UnboundedSender;
use tracing::{info, warn};
use wellbeing_core::DateRange;

use crate::dbus::DaemonPresenceEvent;
use crate::reports::build_reports_viewmodel;

use super::repo::ReportsRepo;

pub struct FlowState {
    pub uid: u32,
    pub selected_range: Arc<RwLock<DateRange>>,
}

/// Refreshes on:
/// - `daily_usage_changed` D-Bus signal
/// - Daemon presence change (reconnect)
/// - Manual refresh trigger
pub fn spawn_reports_flow(
    repo: ReportsRepo,
    state: Arc<FlowState>,
    mut presence_rx: broadcast::Receiver<DaemonPresenceEvent>,
    mut refresh_rx: broadcast::Receiver<()>,
    vm_tx: UnboundedSender<Option<super::ReportsViewModel>>,
) {
    tokio::spawn(async move {
        // bool: true = signal received, false = signal stream ended
        let (signal_tx, mut signal_rx) = tokio::sync::mpsc::unbounded_channel::<bool>();
        let mut proxy_subscribed = false;
        let mut daemon_available = true;

        info!("reports flow started");

        loop {
            if !proxy_subscribed && daemon_available {
                match repo.proxy().await {
                    Ok(p) => {
                        if let Ok(mut stream) = p.receive_daily_usage_changed().await {
                            let tx = signal_tx.clone();
                            tokio::spawn(async move {
                                while stream.next().await.is_some() {
                                    let _ = tx.send(true);
                                }
                                warn!("reports flow: daily_usage_changed signal stream ended");
                                let _ = tx.send(false);
                            });
                        }
                        proxy_subscribed = true;
                    }
                    Err(e) => {
                        warn!("reports flow: cannot create proxy for signals: {e}");
                        tokio::time::sleep(Duration::from_secs(5)).await;
                    }
                }
            }

            let triggered = tokio::select! {
                biased;
                Ok(event) = presence_rx.recv() => {
                    match event {
                        DaemonPresenceEvent::Appeared(_) => {
                            info!("reports: daemon appeared, reconnecting");
                            let ok = repo.bus.re_resolve().await;
                            daemon_available = ok;
                            proxy_subscribed = false;
                            ok
                        }
                        DaemonPresenceEvent::Disappeared => {
                            warn!("reports: daemon disappeared");
                            daemon_available = false;
                            false
                        }
                    }
                }
                Some(alive) = signal_rx.recv() => {
                    if !alive {
                        warn!("reports flow: signal stream lost — will re-subscribe");
                        proxy_subscribed = false;
                    }
                    daemon_available
                }
                Ok(_) = refresh_rx.recv() => {
                    daemon_available
                }
            };

            if !triggered {
                // Daemon offline — keep last ViewModel, don't send None.
                continue;
            }

            let range = *state.selected_range.read().await;
            match repo.fetch_all(state.uid, range).await {
                Ok(data) => {
                    let vm = build_reports_viewmodel(
                        range,
                        &data.summaries,
                        &data.app_categories,
                        &data.title_entries,
                    );
                    let _ = vm_tx.send(Some(vm));
                }
                Err(e) => {
                    warn!("reports: fetch_all failed: {e}");
                }
            }
        }
    });
}
