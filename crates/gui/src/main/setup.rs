use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use tokio::sync::mpsc;
use tokio::time::interval;
use tracing::{info, warn};
use wellbeing_gui::app::{App, AppState, AppViewModels};
use wellbeing_gui::dashboard::BlockCardInfo;
use wellbeing_gui::dbus::{
    self, CoalescedNotifications, ConnectionStatus, DaemonClient, DaemonPresenceEvent,
    SignalCoalescer,
};

/// Connect to daemon and set up signal subscription.
pub async fn setup_daemon_connection() -> (
    DaemonClient,
    mpsc::UnboundedReceiver<CoalescedNotifications>,
    Arc<SignalCoalescer>,
    mpsc::UnboundedSender<CoalescedNotifications>,
    bool,
    ConnectionStatus,
) {
    let (signal_tx, signal_rx) = mpsc::unbounded_channel();
    let coalescer = Arc::new(SignalCoalescer::new());

    match DaemonClient::connect().await {
        Ok(client) => {
            info!("connected to wellbeing daemon");
            let status = client.connection_status();
            dbus::spawn_signal_listener(&client, coalescer.clone(), signal_tx.clone());
            (client, signal_rx, coalescer, signal_tx, true, status)
        }
        Err(e) => {
            warn!("daemon unavailable: {e}");
            // Still connect to both busses for NameOwnerChanged readiness.
            let client = DaemonClient::degraded().await;
            (
                client,
                signal_rx,
                coalescer,
                signal_tx,
                false,
                ConnectionStatus::Disconnected,
            )
        }
    }
}

/// Builds ViewModels after each refresh and emits them through `vm_tx` to the
/// GPUI entity — the foreground half of the StateFlow.
pub async fn background_loop(
    state: Arc<tokio::sync::Mutex<AppState>>,
    mut client: DaemonClient,
    mut signal_rx: mpsc::UnboundedReceiver<CoalescedNotifications>,
    vm_tx: mpsc::UnboundedSender<AppViewModels>,
    coalescer: Arc<SignalCoalescer>,
    signal_tx: mpsc::UnboundedSender<CoalescedNotifications>,
    mut presence_rx: mpsc::UnboundedReceiver<DaemonPresenceEvent>,
) {
    // Periodic fallback refresh — catches missed D-Bus signals or dead
    // signal streams without requiring the user to restart the GUI.
    let mut ticker = interval(std::time::Duration::from_secs(60));
    // Skip the first immediate tick so the initial refresh from startup
    // isn't immediately overwritten by the periodic loop.
    ticker.tick().await;

    loop {
        tokio::select! {
            Some(event) = presence_rx.recv() => {
                let reconnected = client.re_resolve_bus().await;
                let mut s = state.lock().await;
                if reconnected {
                    info!("daemon reconnected after {:?} event", event);
                    s.client = client.clone();
                    s.connection_status = client.connection_status();
                    s.daemon_available = client.connection_status().is_connected();
                    // Clear stale empty caches from degraded mode so the
                    // immediate refresh pulls real data instead of serving
                    // old empty vectors.
                    s.range_cache.clear();
                    s.policy_cache.clear();
                    s.category_cache.clear();
                    s.app_category_cache.clear();
                    s.day_events_cache.clear();
                    s.title_cache.clear();
                    drop(s);
                    dbus::spawn_signal_listener(&client, coalescer.clone(), signal_tx.clone());
                    // Refresh immediately so the UI shows data
                    // without waiting for the next signal.
                    refresh_and_emit(&state, &client, &vm_tx).await;
                } else {
                    // Daemon disappeared — update UI to show disconnected
                    // state even when no daemon is reachable.
                    s.client = client.clone();
                    s.connection_status = client.connection_status();
                    s.daemon_available = false;
                    drop(s);
                    refresh_and_emit(&state, &client, &vm_tx).await;
                }
            }
            Some(notif) = signal_rx.recv() => {
                // Invalidate D-Bus client caches so the next fetch hits the
                // daemon instead of serving stale data. The caller (signal
                // or reconnect) explicitly wants fresh data.
                if notif.usage {
                    client.invalidate_range_cache();
                    client.invalidate_range_by_title_cache();
                    client.invalidate_day_events_cache();
                    client.invalidate_daily_title_cache();
                }
                if notif.policy {
                    client.invalidate_policy_cache();
                }
                let _ = coalescer.drain();
                refresh_and_emit(&state, &client, &vm_tx).await;
            }
            _ = ticker.tick() => {
                // Periodic fallback refresh — ensures the dashboard stays
                // current even if D-Bus signals are missed or streams die.
                client.invalidate_range_cache();
                client.invalidate_day_events_cache();
                client.invalidate_daily_title_cache();
                client.invalidate_policy_cache();
                client.invalidate_category_caches();
                let _ = coalescer.drain();
                refresh_and_emit(&state, &client, &vm_tx).await;
            }
        }
    }
}

async fn refresh_and_emit(
    state: &Arc<tokio::sync::Mutex<AppState>>,
    client: &DaemonClient,
    vm_tx: &mpsc::UnboundedSender<AppViewModels>,
) {
    refresh_all_data(state, client).await;

    let vms = App::refresh_viewmodels(state).await;
    let _ = vm_tx.send(AppViewModels {
        dashboard: vms.0,
        policies: vms.1,
        reports: vms.2,
    });
}

/// Set a state field from a Result, logging a warning on error.
fn set_or_warn<T, E: std::fmt::Display>(
    s: &mut AppState,
    result: Result<T, E>,
    setter: impl FnOnce(&mut AppState, T),
    name: &str,
) {
    match result {
        Ok(value) => setter(s, value),
        Err(e) => warn!(error = %e, "failed to fetch {name}"),
    }
}

pub(crate) async fn refresh_all_data(
    state: &Arc<tokio::sync::Mutex<AppState>>,
    client: &DaemonClient,
) {
    let (uid, range) = {
        let s = state.lock().await;
        (s.uid, s.selected_range)
    };
    let start = range.start_str();
    let end = range.end_str();

    // Day events for today's timeline chart: midnight → midnight tomorrow UTC.
    let today_start = chrono::Utc::now()
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .unwrap();
    let today_end = today_start + Duration::days(1);
    let day_start_ms = today_start.and_utc().timestamp_millis();
    let day_end_ms = today_end.and_utc().timestamp_millis();

    let usage_fut = client.get_usage_range(&start, &end, uid);
    let policy_fut = client.list_policies(uid);
    let cat_fut = client.list_categories();
    let app_cat_fut = client.get_app_categories();
    let blocks_fut = client.get_blocked_apps();
    let day_events_fut = client.get_day_events(uid, day_start_ms, day_end_ms);
    let title_fut = client.get_daily_usage_by_title(&start, uid);

    let (usage, policies, categories, app_categories, blocks, day_events, title) = tokio::join!(
        usage_fut,
        policy_fut,
        cat_fut,
        app_cat_fut,
        blocks_fut,
        day_events_fut,
        title_fut
    );

    let mut s = state.lock().await;
    set_or_warn(&mut s, usage, |s, v| s.range_cache = v, "usage range");
    set_or_warn(&mut s, policies, |s, v| s.policy_cache = v, "policies");
    set_or_warn(
        &mut s,
        categories,
        |s, v| s.category_cache = v,
        "categories",
    );
    set_or_warn(
        &mut s,
        app_categories,
        |s, v| s.app_category_cache = v,
        "app categories",
    );
    set_or_warn(
        &mut s,
        blocks,
        |s, entries| {
            s.block_cards = entries
                .into_iter()
                .map(|b| BlockCardInfo {
                    app_id: b.app_id,
                    display_name: String::new(),
                    blocked_since: DateTime::from_timestamp(b.blocked_since as i64, 0)
                        .unwrap_or(Utc::now()),
                })
                .collect();
        },
        "active blocks",
    );
    set_or_warn(
        &mut s,
        day_events,
        |s, v| s.day_events_cache = v,
        "day events",
    );
    set_or_warn(&mut s, title, |s, v| s.title_cache = v, "title usage");
}
