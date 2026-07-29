//! ReportsFlow — independent background task with signal-triggered refresh,
//! presence-aware reconnection, and timeout-guarded D-Bus fetches.
//!
//! The `ReportsViewModel` persists across updates (like a Compose ViewModel)
//! and is patched in-place rather than rebuilt from a builder function.

use std::sync::Arc;

use futures::StreamExt;
use tokio::sync::RwLock;
use tokio::sync::broadcast;
use tokio::sync::watch;
use tracing::{info, warn};
use wellbeing_core::DateRange;

use crate::dbus::DaemonPresenceEvent;
use crate::reports::ReportsViewModel;

use super::repo::ReportsRepo;

/// Discriminated flow events forwarded from D-Bus signal subscriptions
/// to the main loop, so each signal triggers the correct action.
enum FlowSignal {
    DailyUsageChanged,
    /// One of the signal forwarding streams ended (transient D-Bus glitch).
    /// Triggers re-subscription on the next loop iteration.
    SignalStreamEnded,
}

pub struct FlowState {
    pub uid: u32,
    pub selected_range: Arc<RwLock<DateRange>>,
}

/// Refreshes on:
/// - `daily_usage_changed` D-Bus signal
/// - Daemon presence change (reconnect)
/// - Manual refresh trigger (range change, etc.)
pub fn spawn_reports_flow(
    repo: ReportsRepo,
    state: Arc<FlowState>,
    mut presence_rx: broadcast::Receiver<DaemonPresenceEvent>,
    mut refresh_rx: broadcast::Receiver<()>,
    vm_tx: watch::Sender<Option<ReportsViewModel>>,
) {
    tokio::spawn(async move {
        let (signal_tx, mut signal_rx) = tokio::sync::mpsc::unbounded_channel::<FlowSignal>();
        let mut proxy_subscribed = false;
        let mut daemon_available = true;

        // Persistent ViewModel — like a Compose ViewModel with StateFlow.
        let mut current_vm = ReportsViewModel::default();
        let mut generation: u64 = 0;

        info!("reports flow started");

        loop {
            if !proxy_subscribed && daemon_available {
                match repo.proxy().await {
                    Ok(p) => {
                        if let Ok(mut stream) = p.receive_daily_usage_changed().await {
                            let tx = signal_tx.clone();
                            tokio::spawn(async move {
                                while stream.next().await.is_some() {
                                    let _ = tx.send(FlowSignal::DailyUsageChanged);
                                }
                                warn!("reports flow: daily_usage_changed signal stream ended");
                                let _ = tx.send(FlowSignal::SignalStreamEnded);
                            });
                        }
                        proxy_subscribed = true;
                    }
                    Err(e) => {
                        warn!("reports flow: cannot create proxy for signals: {e}");
                        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    }
                }
            }

            tokio::select! {
                biased;
                Ok(event) = presence_rx.recv() => {
                    match event {
                        DaemonPresenceEvent::Appeared(_) => {
                            info!("reports: daemon appeared, reconnecting");
                            let ok = repo.bus.re_resolve().await;
                            daemon_available = ok;
                            proxy_subscribed = false;
                            if ok {
                                generation += 1;
                                let my_gen = generation;
                                do_full_fetch(&repo, &state, &mut current_vm, &vm_tx, my_gen, &mut generation).await;
                            }
                        }
                        DaemonPresenceEvent::Disappeared => {
                            warn!("reports: daemon disappeared");
                            daemon_available = false;
                        }
                    }
                }
                signal = signal_rx.recv() => {
                    match signal {
                        Some(FlowSignal::DailyUsageChanged) => {
                            if daemon_available {
                                generation += 1;
                                let my_gen = generation;
                                do_full_fetch(&repo, &state, &mut current_vm, &vm_tx, my_gen, &mut generation).await;
                            }
                        }
                        Some(FlowSignal::SignalStreamEnded) => {
                            warn!("reports flow: signal stream lost — will re-subscribe");
                            proxy_subscribed = false;
                        }
                        None => {}
                    }
                }
                Ok(_) = refresh_rx.recv() => {
                                generation += 1;
                                let my_gen = generation;
                                do_full_fetch(&repo, &state, &mut current_vm, &vm_tx, my_gen, &mut generation).await;
                }
            };
        }
    });
}

/// Fetch ALL reports data and update the ViewModel's raw data + derived state.
///
/// Falls back to the last good state on error — the VM is never cleared.
async fn do_full_fetch(
    repo: &ReportsRepo,
    state: &FlowState,
    vm: &mut ReportsViewModel,
    tx: &watch::Sender<Option<ReportsViewModel>>,
    fetch_gen: u64,
    generation: &mut u64,
) {
    let range = *state.selected_range.read().await;
    match repo.fetch_all(state.uid, range).await {
        Ok(data) => {
            if fetch_gen != *generation {
                return;
            }
            vm.data = Some(data);
            vm.recompute_derived(range);
            let _ = tx.send(Some(vm.clone()));
        }
        Err(e) => {
            warn!("reports: fetch_all failed: {e}");
        }
    }
}
