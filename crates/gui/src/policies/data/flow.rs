//! PoliciesFlow — independent background task that listens for D-Bus signals
//! and presence events, fetches fresh data via the repository, builds a
//! ViewModel, and emits it to the GPUI thread.

use std::time::Duration;

use futures::StreamExt;
use tokio::sync::broadcast;
use tokio::sync::mpsc::UnboundedSender;
use tokio::time::sleep;
use tracing::{info, warn};

use wellbeing_core::AppClass;

use crate::dbus::DaemonPresenceEvent;
use crate::policies::build_policies_viewmodel;

use super::repo::PoliciesRepo;

/// Spawn the policies background flow.
///
/// The flow:
/// 1. Listens for `policy_mutated` D-Bus signals, presence events,
///    and manual refresh triggers.
/// 2. Fetches policies + categories via `PoliciesRepo`.
/// 3. Builds the `PoliciesViewModel`.
/// 4. Sends it through `vm_tx` to the GPUI entity.
pub fn spawn_policies_flow(
    repo: PoliciesRepo,
    uid: u32,
    is_admin: bool,
    mut presence_rx: broadcast::Receiver<DaemonPresenceEvent>,
    mut refresh_rx: broadcast::Receiver<()>,
    vm_tx: UnboundedSender<Option<super::PoliciesViewModel>>,
) {
    tokio::spawn(async move {
        // bool: true = signal received, false = signal stream ended
        let (signal_tx, mut signal_rx) = tokio::sync::mpsc::unbounded_channel::<bool>();
        let mut proxy_subscribed = false;

        let mut daemon_available = true;

        info!("policies flow started");

        loop {
            if !proxy_subscribed && daemon_available {
                match repo.proxy().await {
                    Ok(p) => {
                        if let Ok(mut stream) = p.receive_policy_mutated().await {
                            let tx = signal_tx.clone();
                            tokio::spawn(async move {
                                while stream.next().await.is_some() {
                                    let _ = tx.send(true);
                                }
                                warn!("policies flow: policy_mutated signal stream ended");
                                let _ = tx.send(false);
                            });
                        }
                        proxy_subscribed = true;
                    }
                    Err(e) => {
                        warn!("policies flow: cannot create proxy for signals: {e}");
                        sleep(Duration::from_secs(5)).await;
                    }
                }
            }

            let triggered = tokio::select! {
                biased;
                Ok(event) = presence_rx.recv() => {
                    match event {
                        DaemonPresenceEvent::Appeared(_) => {
                            info!("policies: daemon appeared, reconnecting");
                            let ok = repo.bus.re_resolve().await;
                            daemon_available = ok;
                            proxy_subscribed = false;
                            ok
                        }
                        DaemonPresenceEvent::Disappeared => {
                            warn!("policies: daemon disappeared");
                            daemon_available = false;
                            false
                        }
                    }
                }
                Some(alive) = signal_rx.recv() => {
                    if !alive {
                        warn!("policies flow: signal stream lost — will re-subscribe");
                        proxy_subscribed = false;
                    }
                    daemon_available
                }
                Ok(_) = refresh_rx.recv() => {
                    daemon_available
                }
            };

            if !triggered {
                let _ = vm_tx.send(None);
                continue;
            }

            match repo.fetch_all(uid).await {
                Ok(data) => {
                    let app_classs: Vec<AppClass> = data
                        .app_list
                        .iter()
                        .map(|ac| ac.app_class.clone())
                        .collect();
                    let vm = build_policies_viewmodel(
                        &data.policies,
                        &data.categories,
                        &app_classs,
                        is_admin,
                    );
                    let _ = vm_tx.send(Some(vm));
                }
                Err(e) => {
                    warn!("policies: fetch_all failed: {e}");
                }
            }
        }
    });
}
