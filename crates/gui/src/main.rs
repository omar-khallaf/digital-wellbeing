//! wellbeing-gui — Digital Wellbeing Desktop GUI.
//!
//! Startup sequence:
//! 1. Initialize tracing.
//! 2. Connect to daemon via `DaemonClient` (4-step bus resolution).
//! 3. Subscribe to daemon signals.
//! 4. Start background tokio task for signal handling + periodic queries.
//! 5. Run gpui application loop.
//! 6. On daemon unavailable → show warning banner (degraded mode).

use std::sync::Arc;

use gpui::px;
use gpui::*;
use gpui_component::{ActiveTheme, Root, theme::Theme};
use tokio::sync::mpsc;
use tracing::info;
use tracing::warn;

use wellbeing_core::DateRange;
use wellbeing_gui::app::{App, AppState, AppViewModels, RenderMode};
use wellbeing_gui::dbus::{self, CoalescedNotifications, DaemonClient, SignalCoalescer};

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

    let (client, signal_rx, daemon_available) = setup_daemon_connection().await;

    let state = Arc::new(tokio::sync::Mutex::new(AppState {
        uid,
        mode,
        client: client.clone(),
        selected_range: DateRange::last_n_days(7),
        range_cache: Vec::new(),
        policy_cache: Vec::new(),
        category_cache: Vec::new(),
        app_category_cache: Vec::new(),
        block_cards: Vec::new(),
        daemon_available,
    }));

    // Channel: background loop → GPUI entity (StateFlow-like VM events).
    let (vm_tx, vm_rx) = mpsc::unbounded_channel();

    // Spawn background tokio task for signal handling + periodic refresh.
    let bg_state = state.clone();
    let bg_client = client.clone();
    let vm_tx_clone = vm_tx.clone();
    tokio::spawn(async move {
        // Immediate first refresh so the UI doesn't wait 5s for initial data.
        refresh_and_emit(&bg_state, &bg_client, &vm_tx_clone).await;
        background_loop(bg_state, bg_client, signal_rx, vm_tx_clone).await;
    });

    // Run gpui application on the main thread.
    Application::new_inaccessible(gpui_platform::current_platform(false)).run(move |app| {
        // MUST be called before any gpui_component feature is used (Root, Theme,
        // Button, Input, charts, ...). Visible wiring — no hidden init.
        gpui_component::init(app);

        // Respect system dark/light preference.
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

            // Drain VM events from the background loop and apply them to the
            // App entity — this is the foreground half of the StateFlow.
            let entity = app_view.clone();
            std::mem::drop(cx.spawn(async move |cx| {
                while let Some(vms) = vm_rx.recv().await {
                    entity.update(cx, |app, cx| {
                        app.apply_viewmodels(vms);
                        cx.notify();
                    });
                }
            }));

            cx.new(|cx| Root::new(app_view, window, cx).bg(cx.theme().background))
        })
        .expect("failed to open window");
    });
}

/// Connect to daemon and set up signal subscription.
///
/// Returns `(client, signal_rx, daemon_available)`.
async fn setup_daemon_connection() -> (
    DaemonClient,
    mpsc::UnboundedReceiver<CoalescedNotifications>,
    bool,
) {
    let (signal_tx, signal_rx) = mpsc::unbounded_channel();

    match DaemonClient::connect().await {
        Ok(client) => {
            info!("connected to wellbeing daemon");
            let coalescer = Arc::new(SignalCoalescer::new());
            dbus::spawn_signal_listener(&client, coalescer, signal_tx.clone());
            (client, signal_rx, true)
        }
        Err(e) => {
            warn!("daemon unavailable: {e}");
            let conn = match zbus::Connection::session().await {
                Ok(c) => c,
                Err(_) => {
                    let coalescer = Arc::new(SignalCoalescer::new());
                    drop(coalescer);
                    panic!(
                        "daemon unreachable. start daemon first:\n  sudo systemctl start digital-wellbeing-daemon\n  # or: wellbeing-daemon"
                    );
                }
            };
            let client = DaemonClient::degraded(conn).await;
            (client, signal_rx, false)
        }
    }
}

/// Background loop: processes signals + periodic data refresh.
/// Builds ViewModels after each refresh and emits them through `vm_tx` to the
/// GPUI entity — the foreground half of the StateFlow.
async fn background_loop(
    state: Arc<tokio::sync::Mutex<AppState>>,
    client: DaemonClient,
    mut signal_rx: mpsc::UnboundedReceiver<CoalescedNotifications>,
    vm_tx: mpsc::UnboundedSender<AppViewModels>,
) {
    // Periodically refresh data every 5 seconds.
    let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(5));

    loop {
        tokio::select! {
            _ = interval.tick() => {
                refresh_and_emit(&state, &client, &vm_tx).await;
            }
            Some(_notif) = signal_rx.recv() => {
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

/// Refresh all cached data from the daemon.
async fn refresh_all_data(state: &Arc<tokio::sync::Mutex<AppState>>, client: &DaemonClient) {
    let (uid, range) = {
        let s = state.lock().await;
        (s.uid, s.selected_range)
    };
    let start = range.start_str();
    let end = range.end_str();

    // Fetch in parallel using the daemon client.
    let usage_fut = client.get_usage_range(&start, &end, uid);
    let policy_fut = client.list_policies(uid);
    let cat_fut = client.list_categories();
    let app_cat_fut = client.get_app_categories();

    let (usage, policies, categories, app_categories) =
        tokio::join!(usage_fut, policy_fut, cat_fut, app_cat_fut);

    let mut s = state.lock().await;
    if let Ok(entries) = usage {
        s.range_cache = entries;
    }
    if let Ok(policies) = policies {
        s.policy_cache = policies;
    }
    if let Ok(cats) = categories {
        s.category_cache = cats;
    }
    if let Ok(rows) = app_categories {
        s.app_category_cache = rows;
    }
}
