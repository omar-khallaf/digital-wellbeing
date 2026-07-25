//! wellbeing-gui — Digital Wellbeing Desktop GUI.
//!
//! Startup sequence:
//! 1. Initialize tracing.
//! 2. Connect to daemon via `DaemonClient` (4-step bus resolution).
//! 3. Subscribe to daemon signals.
//! 4. Start background tokio task for signal handling + daemon-reconnect resync.
//! 5. Run gpui application loop.
//! 6. On daemon unavailable → show warning banner (degraded mode).

mod setup;

use std::sync::Arc;

use gpui::px;
use gpui::*;
use gpui_component::{ActiveTheme, Root, theme::Theme};
use tokio::sync::mpsc;
use tracing::info;

use wellbeing_core::DateRange;
use wellbeing_gui::app::{App, AppState, RenderMode};
use wellbeing_gui::dbus;

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
        setup::setup_daemon_connection().await;

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
        title_cache: Vec::new(),
        daemon_available,
        connection_status,
    }));

    // Populate state cache before GPUI starts so App::new can build initial
    // ViewModels with real data—no loading-state race.
    setup::refresh_all_data(&state, &client).await;

    // Channel: background loop → GPUI entity (StateFlow-like VM events).
    let (vm_tx, vm_rx) = mpsc::unbounded_channel();

    // Spawn background tokio task for signal handling + daemon-reconnect
    // resync. The initial data population happens above before GPUI starts.
    let bg_state = state.clone();
    let bg_client = client.clone();
    let bg_coalescer = coalescer.clone();
    let bg_signal_tx = signal_tx.clone();
    tokio::spawn(async move {
        setup::background_loop(
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
