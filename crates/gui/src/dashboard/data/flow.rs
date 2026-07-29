//! DashboardFlow — independent background task that listens for D-Bus signals,
//! presence events, and manual refresh triggers, fetches fresh data via the
//! repository, patches the ViewModel in-place, and emits it to the GPUI thread.
//!
//! Each D-Bus signal triggers a targeted re-fetch:
//! - `DailyUsageChanged` → full fetch via `fetch_all`
//! - `BlockedAppsChanged` → blocked apps only, patches `data.blocked`
//!
//! The `DashboardViewModel` persists across updates (like a Compose ViewModel)
//! and is patched in-place rather than rebuilt from a cache — no `cached_data`.

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use futures::StreamExt;
use tokio::sync::broadcast;
use tokio::sync::watch;
use tracing::{info, warn};
use wellbeing_core::DateRange;

use crate::dashboard::domain::DashboardViewModel;
use crate::dbus::DaemonPresenceEvent;
use crate::dbus::client::DaemonProxy;

use super::repo::DashboardRepo;

/// Discriminated flow events forwarded from D-Bus signal subscriptions
/// to the main loop, so each signal triggers the correct action.
enum FlowSignal {
    DailyUsageChanged,
    BlockedAppsChanged,
    /// One of the signal forwarding streams ended (transient D-Bus glitch).
    /// Triggers re-subscription on the next loop iteration.
    SignalStreamEnded,
}

/// Shared application state that the dashboard flow reads.
pub struct FlowState {
    pub uid: u32,
}

/// Spawn the dashboard background flow.
///
/// The flow maintains a persistent `DashboardViewModel` that is patched
/// in-place as D-Bus signals arrive:
/// - `DailyUsageChanged` / daemon reconnect / manual refresh → full fetch
/// - `BlockedAppsChanged` → only `get_blocked_apps()`, patches `data.blocked`
pub fn spawn_dashboard_flow(
    repo: DashboardRepo,
    state: Arc<FlowState>,
    mut presence_rx: broadcast::Receiver<DaemonPresenceEvent>,
    mut refresh_rx: broadcast::Receiver<()>,
    vm_tx: watch::Sender<Option<DashboardViewModel>>,
) {
    tokio::spawn(async move {
        let (signal_tx, mut signal_rx) = tokio::sync::mpsc::unbounded_channel::<FlowSignal>();
        let mut proxy_subscribed = false;
        let mut daemon_available = true;

        // Persistent ViewModel — like a Compose ViewModel with StateFlow.
        // Accumulates data from signal-driven fetches; never rebuilt from cache.
        let mut current_vm = DashboardViewModel::default();
        let mut gen_cnt: u64 = 0;

        info!("dashboard flow started");

        loop {
            if !proxy_subscribed
                && daemon_available
                && let Some(conn) = repo.bus.try_proxy_conn()
            {
                let mut subscribed = false;
                match tokio::time::timeout(Duration::from_secs(5), DaemonProxy::new(&conn)).await {
                    Ok(Ok(p)) => {
                        if let Ok(mut stream) = p.receive_daily_usage_changed().await {
                            let tx = signal_tx.clone();
                            tokio::spawn(async move {
                                while stream.next().await.is_some() {
                                    let _ = tx.send(FlowSignal::DailyUsageChanged);
                                }
                                warn!("dashboard flow: daily_usage_changed signal stream ended");
                                let _ = tx.send(FlowSignal::SignalStreamEnded);
                            });
                            subscribed = true;
                        }
                        if let Ok(mut stream) = p.receive_on_blocked_apps_changed().await {
                            let tx = signal_tx.clone();
                            tokio::spawn(async move {
                                while stream.next().await.is_some() {
                                    let _ = tx.send(FlowSignal::BlockedAppsChanged);
                                }
                                warn!("dashboard flow: blocked_apps_changed signal stream ended");
                                let _ = tx.send(FlowSignal::SignalStreamEnded);
                            });
                            subscribed = true;
                        }
                        proxy_subscribed = subscribed;
                    }
                    Ok(Err(e)) => {
                        warn!("dashboard flow: proxy for signals failed: {e}");
                    }
                    Err(_) => {
                        warn!("dashboard flow: proxy for signals timed out");
                    }
                }
            }

            tokio::select! {
                biased;
                Ok(event) = presence_rx.recv() => {
                    match event {
                        DaemonPresenceEvent::Appeared(_) => {
                            info!("dashboard: daemon appeared, reconnecting");
                            let ok = repo.bus.re_resolve().await;
                            daemon_available = ok;
                            proxy_subscribed = false;
                            if ok {
                                gen_cnt += 1;
                                let my_gen = gen_cnt;
                                do_full_fetch(&repo, state.uid, &mut current_vm, &vm_tx, my_gen, &mut gen_cnt).await;
                            }
                        }
                        DaemonPresenceEvent::Disappeared => {
                            warn!("dashboard: daemon disappeared");
                            daemon_available = false;
                        }
                    }
                }
                signal = signal_rx.recv() => {
                    match signal {
                        Some(FlowSignal::DailyUsageChanged) => {
                            if daemon_available {
                                gen_cnt += 1;
                                let my_gen = gen_cnt;
                                do_full_fetch(&repo, state.uid, &mut current_vm, &vm_tx, my_gen, &mut gen_cnt).await;
                            }
                        }
                        Some(FlowSignal::BlockedAppsChanged) => {
                            if daemon_available {
                                match repo.get_blocked_apps().await {
                                    Ok(blocked) => {
                                        if let Some(ref mut data) = current_vm.data {
                                            data.blocked = blocked;
                                            current_vm.recompute_blocked();
                                            let _ = vm_tx.send(Some(current_vm.clone()));
                                        }
                                    }
                                    Err(e) => warn!("dashboard: blocked fetch failed: {e}"),
                                }
                            }
                        }
                        Some(FlowSignal::SignalStreamEnded) => {
                            warn!("dashboard flow: signal stream lost — will re-subscribe");
                            proxy_subscribed = false;
                        }
                        None => {}
                    }
                }
                Ok(_) = refresh_rx.recv() => {
                                gen_cnt += 1;
                                let my_gen = gen_cnt;
                                do_full_fetch(&repo, state.uid, &mut current_vm, &vm_tx, my_gen, &mut gen_cnt).await;
                }
            };
        }
    });
}

/// Fetch ALL dashboard data and update the ViewModel's raw data + derived state.
///
/// Falls back to the last good state on error — the VM is never cleared.
async fn do_full_fetch(
    repo: &DashboardRepo,
    uid: u32,
    vm: &mut DashboardViewModel,
    tx: &watch::Sender<Option<DashboardViewModel>>,
    fetch_gen: u64,
    gen_cnt: &mut u64,
) {
    let today = Utc::now().date_naive();
    match repo
        .fetch_all(
            uid,
            DateRange {
                start: today,
                end: today,
            },
        )
        .await
    {
        Ok(data) => {
            // STALENESS CHECK: if a newer signal arrived, discard this result
            if fetch_gen != *gen_cnt {
                return;
            }
            vm.data = Some(data);
            vm.recompute_derived();
            let _ = tx.send(Some(vm.clone()));
        }
        Err(e) => {
            warn!("dashboard: fetch_all failed: {e}");
        }
    }
}
