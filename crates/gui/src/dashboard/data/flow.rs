//! DashboardFlow — independent background task that listens for D-Bus signals,
//! periodic ticks, and presence events, fetches fresh data via the repository,
//! builds a ViewModel, and emits it to the GPUI thread.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, NaiveDate, Utc};
use futures::StreamExt;
use tokio::sync::broadcast;
use tokio::sync::mpsc::UnboundedSender;
use tokio::time::interval;
use tracing::{info, warn};
use wellbeing_core::DateRange;

use crate::dashboard::domain::{BlockCardInfo, DashboardViewModel};
use crate::dashboard::timeline::build_day_timeline;
use crate::dashboard::viewmodel::build_dashboard_viewmodel;
use crate::dbus::DaemonPresenceEvent;
use crate::dbus::client::DaemonProxy;

use super::repo::DashboardRepo;

/// Shared application state that the dashboard flow reads.
pub struct FlowState {
    pub uid: u32,
}

/// Spawn the dashboard background flow.
///
/// The flow:
/// 1. Waits for trigger events (D-Bus signals, ticker, presence, manual refresh)
/// 2. Fetches all dashboard data via `DashboardRepo` (with per-call timeouts)
/// 3. Builds the `DashboardViewModel`
/// 4. Sends it through `vm_tx` to the GPUI entity
pub fn spawn_dashboard_flow(
    repo: DashboardRepo,
    state: Arc<FlowState>,
    mut presence_rx: broadcast::Receiver<DaemonPresenceEvent>,
    mut refresh_rx: broadcast::Receiver<()>,
    vm_tx: UnboundedSender<Option<DashboardViewModel>>,
) {
    tokio::spawn(async move {
        let mut ticker = interval(Duration::from_secs(60));
        ticker.tick().await; // skip first immediate tick

        let (signal_tx, mut signal_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut proxy_subscribed = false;

        let mut daemon_available = true;

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
                                    let _ = tx.send(());
                                }
                            });
                        }
                        if let Ok(mut stream) = p.receive_on_blocked_apps_changed().await {
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
                        warn!("dashboard flow: proxy for signals failed: {e}");
                    }
                }
            }

            let triggered = tokio::select! {
                biased;
                Ok(event) = presence_rx.recv() => {
                    match event {
                        DaemonPresenceEvent::Appeared(_) => {
                            info!("dashboard: daemon appeared, reconnecting");
                            let ok = repo.bus.re_resolve().await;
                            daemon_available = ok;
                            proxy_subscribed = false; // re-subscribe
                            ok
                        }
                        DaemonPresenceEvent::Disappeared => {
                            warn!("dashboard: daemon disappeared");
                            daemon_available = false;
                            false
                        }
                    }
                }
                _ = ticker.tick() => {
                    daemon_available
                }
                Some(_) = signal_rx.recv() => {
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

            // Fetch fresh data with timeouts — dashboard always shows today.
            let today: NaiveDate = chrono::Utc::now().date_naive();
            let range = DateRange {
                start: today,
                end: today,
            };
            match repo.fetch_all(state.uid, range).await {
                Ok(data) => {
                    let block_cards: Vec<BlockCardInfo> = data
                        .blocked
                        .iter()
                        .map(|b| BlockCardInfo {
                            app_id: b.app_id.clone(),
                            display_name: String::new(),
                            blocked_since: DateTime::from_timestamp(b.blocked_since as i64, 0)
                                .unwrap_or(Utc::now()),
                        })
                        .collect();

                    let app_names: HashMap<String, String> = data
                        .app_categories
                        .iter()
                        .map(|ac| (ac.app_id.clone(), ac.display_name.clone()))
                        .collect();

                    let day_timeline = Some(build_day_timeline(data.day_events, today, &app_names));

                    let vm = build_dashboard_viewmodel(
                        range,
                        &data.summaries,
                        &data.categories,
                        &data.app_categories,
                        block_cards,
                        day_timeline,
                        &data.title_entries,
                    );
                    let _ = vm_tx.send(Some(vm));
                }
                Err(e) => {
                    warn!("dashboard: fetch_all failed: {e}");
                }
            }
        }
    });
}
