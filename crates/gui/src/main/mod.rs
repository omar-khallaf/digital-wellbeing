//! wellbeing-gui — Digital Wellbeing Desktop GUI.
//!
//! Startup sequence:
//! 1. Initialize tracing.
//! 2. Connect to both busses via `BusManager` and detect daemon presence.
//! 3. Spawn daemon presence broadcast watcher.
//! 4. Create per-screen repositories.
//! 5. Create shared `AppState` (connection info, selected range).
//! 6. Create broadcast channels for per-flow manual refresh triggers.
//! 7. Spawn three background flows (dashboard, policies, reports) with
//!    independent D-Bus signal subscriptions and periodic refresh.
//! 8. Start gpui application loop — the flows push ViewModel updates
//!    through mpsc channels, merged into `AppViewModels` bundles.
//! 9. On daemon unavailable → each flow emits `None` → warning banner.

use std::sync::Arc;

use gpui::px;
use gpui::*;
use gpui_component::{ActiveTheme, Root, theme::Theme};
use tokio::sync::RwLock;
use tokio::sync::broadcast;
use tokio::sync::mpsc;
use tracing::info;
use wellbeing_core::DateRange;

use wellbeing_gui::app::{App, AppState, AppViewModels, RenderMode};
use wellbeing_gui::dashboard::data::{
    DashboardRepo, FlowState as DashFlowState, spawn_dashboard_flow,
};
use wellbeing_gui::dbus;
use wellbeing_gui::dbus::BusManager;
use wellbeing_gui::dbus::DaemonPresenceEvent;
use wellbeing_gui::policies::data::{PoliciesRepo, spawn_policies_flow};
use wellbeing_gui::reports::data::{FlowState as RepFlowState, ReportsRepo, spawn_reports_flow};

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

    // 1. BusManager — dual-bus connections with daemon resolution
    let bus = BusManager::connect().await;
    let daemon_available = Arc::new(RwLock::new(bus.status().await.is_connected()));
    let connection_status = Arc::new(RwLock::new(bus.status().await));

    // 2. Daemon presence broadcast — instant (dis)appearance detection
    let presence_rx =
        dbus::spawn_daemon_presence_broadcast(&bus.system_connection(), &bus.session_connection());

    // 3. Repositories (timeout-guarded D-Bus access)
    let dash_repo = DashboardRepo::new(bus.clone());
    let policies_repo = PoliciesRepo::new(bus.clone());
    let reports_repo = ReportsRepo::new(bus);

    // 4. Shared state
    let selected_range = Arc::new(RwLock::new(DateRange::last_n_days(7)));
    let state = Arc::new(AppState {
        mode,
        uid,
        selected_range: selected_range.clone(),
        daemon_available: daemon_available.clone(),
        connection_status: connection_status.clone(),
    });

    // 5. Daemon presence → AppState connection status
    {
        let daemon_available = daemon_available.clone();
        let connection_status = connection_status.clone();
        let mut presence_rx = presence_rx.resubscribe();
        tokio::spawn(async move {
            use tokio::sync::broadcast::error::RecvError;
            loop {
                match presence_rx.recv().await {
                    Ok(DaemonPresenceEvent::Appeared(bt)) => {
                        *daemon_available.write().await = true;
                        *connection_status.write().await = dbus::ConnectionStatus::Connected(bt);
                    }
                    Ok(DaemonPresenceEvent::Disappeared) => {
                        *daemon_available.write().await = false;
                        *connection_status.write().await = dbus::ConnectionStatus::Disconnected;
                    }
                    Err(RecvError::Closed) => break,
                    Err(RecvError::Lagged(_)) => continue,
                }
            }
        });
    }

    // 6. Broadcast channels for per-flow manual refresh triggers
    let (dash_refresh_tx, dash_refresh_rx) = broadcast::channel::<()>(16);
    let (pol_refresh_tx, pol_refresh_rx) = broadcast::channel::<()>(16);
    let (rep_refresh_tx, rep_refresh_rx) = broadcast::channel::<()>(16);

    // 6. Per-flow ViewModel channels + merge task
    //    Each flow pushes its own ViewModel type; a merge task combines
    //    them into AppViewModels bundles for the GPUI entity.
    let (dash_vm_tx, mut dash_vm_rx) = mpsc::unbounded_channel();
    let (pol_vm_tx, mut pol_vm_rx) = mpsc::unbounded_channel();
    let (rep_vm_tx, mut rep_vm_rx) = mpsc::unbounded_channel();
    let (vm_tx, mut vm_rx) = mpsc::unbounded_channel();

    tokio::spawn(async move {
        let (mut dashboard, mut policies, mut reports) = (None, None, None);
        loop {
            tokio::select! {
                Some(vm) = dash_vm_rx.recv() => { dashboard = vm; }
                Some(vm) = pol_vm_rx.recv() => { policies = vm; }
                Some(vm) = rep_vm_rx.recv() => { reports = vm; }
                else => break,
            }
            let _ = vm_tx.send(AppViewModels {
                dashboard: dashboard.clone(),
                policies: policies.clone(),
                reports: reports.clone(),
            });
        }
    });

    // 7. Spawn background flows (one per screen)
    let presence_rx1 = presence_rx.resubscribe();
    let presence_rx2 = presence_rx.resubscribe();
    let presence_rx3 = presence_rx.resubscribe();

    let dash_flow_state = Arc::new(DashFlowState { uid });
    spawn_dashboard_flow(
        dash_repo,
        dash_flow_state,
        presence_rx1,
        dash_refresh_rx,
        dash_vm_tx,
    );

    spawn_policies_flow(
        policies_repo.clone(),
        uid,
        mode.is_admin(),
        presence_rx2,
        pol_refresh_rx,
        pol_vm_tx,
    );

    let rep_flow_state = Arc::new(RepFlowState {
        uid,
        selected_range: selected_range.clone(),
    });
    spawn_reports_flow(
        reports_repo,
        rep_flow_state,
        presence_rx3,
        rep_refresh_rx,
        rep_vm_tx,
    );

    // Kick all three flows with an initial refresh signal so the UI
    // populates immediately instead of waiting for the first D-Bus
    // signal or periodic ticker to fire.
    let _ = dash_refresh_tx.send(());
    let _ = pol_refresh_tx.send(());
    let _ = rep_refresh_tx.send(());

    Application::new_inaccessible(gpui_platform::current_platform(false)).run(move |app| {
        gpui_component::init(app);
        Theme::sync_system_appearance(None, app);

        // Store the reports refresh sender on App so range-change callbacks
        // can trigger a re-fetch.
        let reports_refresh_tx = rep_refresh_tx.clone();

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
            let app_view = cx.new(|_cx| {
                let mut a = App::new(state.clone());
                a.set_policies_repo(policies_repo);
                a.set_reports_refresh_tx(reports_refresh_tx);
                a.set_pol_refresh_tx(pol_refresh_tx);
                a
            });

            // Wire up the ViewModel receiver — each AppViewModels bundle
            // updates the entity and triggers a re-render.
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
