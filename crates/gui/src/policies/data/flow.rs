//! PoliciesFlow — independent background task that listens for D-Bus signals
//! and presence events, fetches fresh data via the repository, patches the
//! ViewModel in-place, and emits it to the GPUI thread.
//!
//! The `PoliciesViewModel` persists across updates (like a Compose ViewModel)
//! and is patched in-place rather than rebuilt from a builder function.

use futures::StreamExt;
use tokio::sync::broadcast;
use tokio::sync::mpsc::UnboundedSender;
use tracing::{info, warn};

use crate::dbus::DaemonPresenceEvent;
use crate::policies::PoliciesViewModel;

use super::repo::PoliciesRepo;

/// Discriminated flow events forwarded from D-Bus signal subscriptions
/// to the main loop, so each signal triggers the correct action.
enum FlowSignal {
    PolicyMutated,
    /// One of the signal forwarding streams ended (transient D-Bus glitch).
    /// Triggers re-subscription on the next loop iteration.
    SignalStreamEnded,
}

/// Spawn the policies background flow.
///
/// The flow maintains a persistent `PoliciesViewModel` that is refreshed
/// on any trigger (policy_mutated signal / daemon reconnect / manual refresh).
pub fn spawn_policies_flow(
    repo: PoliciesRepo,
    uid: u32,
    is_admin: bool,
    mut presence_rx: broadcast::Receiver<DaemonPresenceEvent>,
    mut refresh_rx: broadcast::Receiver<()>,
    vm_tx: UnboundedSender<Option<PoliciesViewModel>>,
) {
    tokio::spawn(async move {
        let (signal_tx, mut signal_rx) = tokio::sync::mpsc::unbounded_channel::<FlowSignal>();
        let mut proxy_subscribed = false;
        let mut daemon_available = true;

        // Persistent ViewModel — like a Compose ViewModel with StateFlow.
        let mut current_vm = PoliciesViewModel::default();
        current_vm.is_admin = is_admin;

        info!("policies flow started");

        loop {
            if !proxy_subscribed && daemon_available {
                match repo.proxy().await {
                    Ok(p) => {
                        if let Ok(mut stream) = p.receive_policy_mutated().await {
                            let tx = signal_tx.clone();
                            tokio::spawn(async move {
                                while stream.next().await.is_some() {
                                    let _ = tx.send(FlowSignal::PolicyMutated);
                                }
                                warn!("policies flow: policy_mutated signal stream ended");
                                let _ = tx.send(FlowSignal::SignalStreamEnded);
                            });
                        }
                        proxy_subscribed = true;
                    }
                    Err(e) => {
                        warn!("policies flow: cannot create proxy for signals: {e}");
                        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    }
                }
            }

            tokio::select! {
                biased;
                Ok(event) = presence_rx.recv() => {
                    match event {
                        DaemonPresenceEvent::Appeared(_) => {
                            info!("policies: daemon appeared, reconnecting");
                            let ok = repo.bus.re_resolve().await;
                            daemon_available = ok;
                            proxy_subscribed = false;
                            if ok {
                                do_full_fetch(&repo, uid, &mut current_vm, &vm_tx).await;
                            }
                        }
                        DaemonPresenceEvent::Disappeared => {
                            warn!("policies: daemon disappeared");
                            daemon_available = false;
                        }
                    }
                }
                signal = signal_rx.recv() => {
                    match signal {
                        Some(FlowSignal::PolicyMutated) => {
                            if daemon_available {
                                do_full_fetch(&repo, uid, &mut current_vm, &vm_tx).await;
                            }
                        }
                        Some(FlowSignal::SignalStreamEnded) => {
                            warn!("policies flow: signal stream lost — will re-subscribe");
                            proxy_subscribed = false;
                        }
                        None => {}
                    }
                }
                Ok(_) = refresh_rx.recv() => {
                    if daemon_available {
                        do_full_fetch(&repo, uid, &mut current_vm, &vm_tx).await;
                    }
                }
            };
        }
    });
}

/// Fetch ALL policies data and update the ViewModel's raw data + derived state.
///
/// Falls back to the last good state on error — the VM is never cleared.
async fn do_full_fetch(
    repo: &PoliciesRepo,
    uid: u32,
    vm: &mut PoliciesViewModel,
    tx: &UnboundedSender<Option<PoliciesViewModel>>,
) {
    match repo.fetch_all(uid).await {
        Ok(data) => {
            vm.data = Some(data);
            vm.recompute_derived();
            let _ = tx.send(Some(vm.clone()));
        }
        Err(e) => {
            warn!("policies: fetch_all failed: {e}");
        }
    }
}
