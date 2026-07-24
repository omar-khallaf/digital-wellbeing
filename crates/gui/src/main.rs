//! wellbeing-gui — Digital Wellbeing Desktop GUI.
//!
//! Startup sequence:
//! 1. Initialize tracing.
//! 2. Connect to daemon via `DaemonClient` (4-step bus resolution).
//! 3. Subscribe to daemon signals.
//! 4. Start background tokio task for signal handling + daemon-reconnect resync.
//! 5. Run gpui application loop.
//! 6. On daemon unavailable → show warning banner (degraded mode).

use std::sync::Arc;

use gpui::px;
use gpui::*;
use gpui_component::{ActiveTheme, Root, theme::Theme};
use tokio::sync::mpsc;
use tracing::{info, warn};

use chrono::{DateTime, Duration, Utc};
use wellbeing_core::DateRange;
use wellbeing_gui::app::{App, AppState, AppViewModels, RenderMode};
use wellbeing_gui::dashboard::BlockCardInfo;
use wellbeing_gui::dbus::{
    self, CoalescedNotifications, ConnectionStatus, DaemonClient, DaemonPresenceEvent,
    SignalCoalescer,
};

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "wellbeing_gui=info,warn".into()),
        )
        .init();

    info!("wellbeing-gui starting");

    let mode = RenderMode::detect();
    let uid = nix::unistd::Uid::current().as_raw();
    info!(mode = ?mode, uid, "GUI starting");

    let (client, signal_rx, coalescer, signal_tx, daemon_available, connection_status) =
        setup_daemon_connection().await;

    // Spawn NameOwnerChanged watchers on both busses for instant daemon
    // (dis)appearance detection — replaces the 10s polling approach.
    let presence_rx =
        dbus::spawn_daemon_name_watch(client.system_connection(), client.session_connection());

    let state = Arc::new(tokio::sync::Mutex::new(AppState {
        uid,
        mode,
        client: client.clone(),
        selected_range: DateRange::last_n_days(1),
        range_cache: Vec::new(),
        policy_cache: Vec::new(),
        category_cache: Vec::new(),
        app_category_cache: Vec::new(),
        block_cards: Vec::new(),
        day_events_cache: Vec::new(),
        daemon_available,
        connection_status,
    }));

    // Populate state cache before GPUI starts so App::new can build initial
    // ViewModels with real data—no loading-state race.
    refresh_all_data(&state, &client).await;

    // Channel: background loop → GPUI entity (StateFlow-like VM events).
    let (vm_tx, vm_rx) = mpsc::unbounded_channel();

    // Spawn background tokio task for signal handling + daemon-reconnect
    // resync. The initial data population happens above before GPUI starts.
    let bg_state = state.clone();
    let bg_client = client.clone();
    let bg_coalescer = coalescer.clone();
    let bg_signal_tx = signal_tx.clone();
    tokio::spawn(async move {
        background_loop(
            bg_state,
            bg_client,
            signal_rx,
            vm_tx,
            bg_coalescer,
            bg_signal_tx,
            presence_rx,
        )
        .await;
    });

    // Run gpui application on the main thread.
    Application::new_inaccessible(gpui_platform::current_platform(false)).run(move |app| {
        // MUST be called before any gpui_component feature is used (Root, Theme,
        // Button, Input, charts, ...). Visible wiring — no hidden init.
        gpui_component::init(app);

        Theme::sync_system_appearance(None, app);

        let state = state.clone();
        let mut vm_rx = vm_rx;
        let window_bounds = WindowBounds::centered(size(px(1000.), px(720.)), app);
        let window_options = WindowOptions {
            window_bounds: Some(window_bounds),
            kind: WindowKind::Normal,
            is_movable: true,
            is_resizable: true,
            is_minimizable: true,
            focus: true,
            show: true,
            window_background: WindowBackgroundAppearance::Opaque,
            ..Default::default()
        };

        app.open_window(window_options, move |window, cx| {
            let app_view = cx.new(|_cx| App::new(state.clone()));

            // MUST store the Task handle in the entity — dropping it cancels
            // the future (including vm_rx) before it ever processes a message.
            let entity = app_view.clone();
            let task = cx.spawn(async move |cx| {
                while let Some(vms) = vm_rx.recv().await {
                    entity.update(cx, |app, cx| {
                        app.apply_viewmodels(vms);
                        cx.notify();
                    });
                }
            });
            app_view.update(cx, |app, _cx| {
                app.set_viewmodel_task(task);
            });

            cx.new(|cx| Root::new(app_view, window, cx).bg(cx.theme().background))
        })
        .expect("failed to open window");
    });
}

/// Connect to daemon and set up signal subscription.
///
/// Returns `(client, signal_rx, coalescer, signal_tx, daemon_available, connection_status)`.
async fn setup_daemon_connection() -> (
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

/// Background loop: processes signals + daemon-reconnect resync.
/// Builds ViewModels after each refresh and emits them through `vm_tx` to the
/// GPUI entity — the foreground half of the StateFlow.
async fn background_loop(
    state: Arc<tokio::sync::Mutex<AppState>>,
    mut client: DaemonClient,
    mut signal_rx: mpsc::UnboundedReceiver<CoalescedNotifications>,
    vm_tx: mpsc::UnboundedSender<AppViewModels>,
    coalescer: Arc<SignalCoalescer>,
    signal_tx: mpsc::UnboundedSender<CoalescedNotifications>,
    mut presence_rx: mpsc::UnboundedReceiver<DaemonPresenceEvent>,
) {
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
                    client.invalidate_day_events_cache();
                }
                if notif.policy {
                    client.invalidate_policy_cache();
                }
                let _ = coalescer.drain();
                refresh_and_emit(&state, &client, &vm_tx).await;
            }
        }
    }
}

/// Fetch fresh data from the daemon, rebuild all ViewModels, and emit them to
/// the GPUI foreground.
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

async fn refresh_all_data(state: &Arc<tokio::sync::Mutex<AppState>>, client: &DaemonClient) {
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
    let blocks_fut = client.get_active_blocks();
    let day_events_fut = client.get_day_events(uid, day_start_ms, day_end_ms);

    let (usage, policies, categories, app_categories, blocks, day_events) = tokio::join!(
        usage_fut,
        policy_fut,
        cat_fut,
        app_cat_fut,
        blocks_fut,
        day_events_fut
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
}
