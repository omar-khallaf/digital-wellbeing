//! DashboardFlow — independent background task that listens for D-Bus signals,
//! presence events, and manual refresh triggers, fetches fresh data via the
//! repository, builds a ViewModel, and emits it to the GPUI thread.
//!
//! Active blocks derive from the `BlockedApps` D-Bus property (source of truth)
//! and are kept up-to-date via the `BlockedAppsChanged` signal — when the signal
//! fires, only the blocked-apps property is re-fetched (not the full dashboard),
//! then the viewmodel is rebuilt from cached data.
//!
//! Relies on the daemon's `DailyUsageChanged` signal (emitted every minute) to
//! drive full-data re-fetches — no polling ticker needed.

use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, NaiveDate, Utc};
use futures::StreamExt;
use tokio::sync::broadcast;
use tokio::sync::mpsc::UnboundedSender;
use tracing::{info, warn};
use wellbeing_core::{BlockedAppEntry, DateRange};

use crate::dashboard::domain::{BlockCardInfo, DashboardViewModel};
use crate::dashboard::timeline::build_day_timeline;
use crate::dashboard::viewmodel::build_dashboard_viewmodel;
use crate::dbus::DaemonPresenceEvent;
use crate::dbus::client::DaemonProxy;

use super::repo::{DashboardData, DashboardRepo};

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
/// The flow:
/// 1. Waits for trigger events (D-Bus signals, presence, manual refresh)
/// 2. Maintains a local `blocked_entries` cache from the `BlockedApps` property
///    and keeps it in sync by re-fetching the property on every
///    `BlockedAppsChanged` signal (without re-fetching the full dashboard)
/// 3. Fetches full dashboard data via `DashboardRepo` only on
///    `DailyUsageChanged` / presence / manual refresh
/// 4. Builds the `DashboardViewModel` and sends it through `vm_tx`
pub fn spawn_dashboard_flow(
    repo: DashboardRepo,
    state: Arc<FlowState>,
    mut presence_rx: broadcast::Receiver<DaemonPresenceEvent>,
    mut refresh_rx: broadcast::Receiver<()>,
    vm_tx: UnboundedSender<Option<DashboardViewModel>>,
) {
    tokio::spawn(async move {
        let (signal_tx, mut signal_rx) = tokio::sync::mpsc::unbounded_channel::<FlowSignal>();
        let mut proxy_subscribed = false;

        let mut daemon_available = true;

        // Local blocked-apps cache — populated from the `BlockedApps` property
        // and refreshed on every `BlockedAppsChanged` signal.
        let mut blocked_entries: Vec<BlockedAppEntry> = Vec::new();

        // Full-data cache — used to rebuild the viewmodel when only the blocked
        // apps state changes, avoiding a full D-Bus re-fetch.
        let mut cached_data: Option<DashboardData> = None;

        info!("dashboard flow started");

        loop {
            // (Re-)subscribe to signals on startup and after reconnect
            if !proxy_subscribed
                && daemon_available
                && let Some(conn) = repo.bus.try_proxy_conn()
            {
                match DaemonProxy::new(&conn).await {
                    Ok(p) => {
                        if let Ok(mut stream) = p.receive_daily_usage_changed().await {
                            let tx = signal_tx.clone();
                            tokio::spawn(async move {
                                while stream.next().await.is_some() {
                                    let _ = tx.send(FlowSignal::DailyUsageChanged);
                                }
                                warn!("dashboard flow: daily_usage_changed signal stream ended");
                                let _ = tx.send(FlowSignal::SignalStreamEnded);
                            });
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
                        }
                        proxy_subscribed = true;
                    }
                    Err(e) => {
                        warn!("dashboard flow: proxy for signals failed: {e}");
                    }
                }
            }

            let mut do_full_refresh = false;
            let mut do_blocked_refresh = false;

            tokio::select! {
                biased;
                Ok(event) = presence_rx.recv() => {
                    match event {
                        DaemonPresenceEvent::Appeared(_) => {
                            info!("dashboard: daemon appeared, reconnecting");
                            let ok = repo.bus.re_resolve().await;
                            daemon_available = ok;
                            proxy_subscribed = false; // re-subscribe
                            do_full_refresh = ok;
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
                            do_full_refresh = daemon_available;
                        }
                        Some(FlowSignal::BlockedAppsChanged) => {
                            do_blocked_refresh = daemon_available;
                        }
                        Some(FlowSignal::SignalStreamEnded) => {
                            warn!("dashboard flow: signal stream lost — will re-subscribe");
                            proxy_subscribed = false;
                        }
                        None => {}
                    }
                }
                Ok(_) = refresh_rx.recv() => {
                    do_full_refresh = daemon_available;
                }
            };

            // Handle blocked-apps-only refresh — re-fetch property, rebuild
            // viewmodel from cached full data, and emit.
            if do_blocked_refresh {
                if let Ok(entries) = repo.get_blocked_apps().await {
                    blocked_entries = entries;
                }

                if let Some(ref data) = cached_data {
                    let block_cards: Vec<BlockCardInfo> = blocked_entries
                        .iter()
                        .map(|b| BlockCardInfo {
                            app_class: b.app_class.to_string(),
                            display_name: String::new(),
                            blocked_since: DateTime::from_timestamp(b.blocked_since as i64, 0)
                                .unwrap_or(Utc::now()),
                        })
                        .collect();

                    let vm = build_dashboard_viewmodel(
                        DateRange {
                            start: Utc::now().date_naive(),
                            end: Utc::now().date_naive(),
                        },
                        data,
                        block_cards,
                        None,
                    );
                    let _ = vm_tx.send(Some(vm));
                }

                // do_full_refresh stays false — skip the full re-fetch below.
            }

            if !do_full_refresh {
                // Daemon offline — keep last ViewModel, don't send None.
                continue;
            }

            // Full data re-fetch — dashboard always shows today.
            let today: NaiveDate = chrono::Utc::now().date_naive();
            let range = DateRange {
                start: today,
                end: today,
            };
            match repo.fetch_all(state.uid, range).await {
                Ok(mut data) => {
                    blocked_entries = data.blocked.clone();

                    let block_cards: Vec<BlockCardInfo> = blocked_entries
                        .iter()
                        .map(|b| BlockCardInfo {
                            app_class: b.app_class.to_string(),
                            display_name: String::new(),
                            blocked_since: DateTime::from_timestamp(b.blocked_since as i64, 0)
                                .unwrap_or(Utc::now()),
                        })
                        .collect();

                    let app_names: HashMap<String, String> = data
                        .app_categories
                        .iter()
                        .map(|ac| (ac.app_class.to_string(), ac.display_name.clone()))
                        .collect();

                    let day_timeline =
                        Some(build_day_timeline(&mut data.day_events, today, &app_names));

                    let vm = build_dashboard_viewmodel(range, &data, block_cards, day_timeline);

                    // Cache full data for incremental blocked-apps updates.
                    cached_data = Some(data);

                    let _ = vm_tx.send(Some(vm));
                }
                Err(e) => {
                    warn!("dashboard: fetch_all failed: {e}");
                }
            }
        }
    });
}
