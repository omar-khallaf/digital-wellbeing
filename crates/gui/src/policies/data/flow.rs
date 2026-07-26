//! PoliciesFlow — independent background task that listens for D-Bus signals
//! and presence events, fetches fresh data via the repository, builds a
//! ViewModel, and emits it to the GPUI thread.

use std::time::Duration;

use futures::StreamExt;
use tokio::sync::broadcast;
use tokio::sync::mpsc::UnboundedSender;
use tokio::time::{interval, sleep};
use tracing::{info, warn};

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
        let (signal_tx, mut signal_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut proxy_subscribed = false;

        let mut daemon_available = true;

        info!("policies flow started");

        let mut fallback_ticker = interval(Duration::from_secs(300));

        loop {
            if !proxy_subscribed && daemon_available {
                match repo.proxy().await {
                    Ok(p) => {
                        if let Ok(mut stream) = p.receive_policy_mutated().await {
                            let tx = signal_tx.clone();
                            tokio::spawn(async move {
                                while stream.next().await.is_some() {
                                    let _ = tx.send(());
                                }
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
                Some(_) = signal_rx.recv() => {
                    daemon_available
                }
                Ok(_) = refresh_rx.recv() => {
                    daemon_available
                }
                _ = fallback_ticker.tick() => {
                    daemon_available
                }
            };

            if !triggered {
                let _ = vm_tx.send(None);
                continue;
            }

            match repo.fetch_all(uid).await {
                Ok(data) => {
                    let app_ids: Vec<String> =
                        data.app_list.iter().map(|ac| ac.app_id.clone()).collect();
                    let vm = build_policies_viewmodel(
                        &data.policies,
                        &data.categories,
                        &app_ids,
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
