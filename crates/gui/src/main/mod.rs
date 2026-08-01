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
//!    through watch channels directly to independent gpui receivers.
//! 9. On daemon unavailable → each flow emits `None` → warning banner.

use std::sync::Arc;

use gpui::px;
use gpui::*;
use gpui_component::{ActiveTheme, Root, theme::Theme};
use tokio::sync::RwLock;
use tokio::sync::broadcast;
use tokio::sync::watch;
use tracing::info;
use wellbeing_core::DateRange;

use wellbeing_gui::app::{App, AppState, RenderMode};
use wellbeing_gui::dashboard::DashboardViewModel;
use wellbeing_gui::dashboard::data::{
    DashboardRepo, FlowState as DashFlowState, spawn_dashboard_flow,
};
use wellbeing_gui::dbus;
use wellbeing_gui::dbus::BusManager;
use wellbeing_gui::dbus::DaemonPresenceEvent;
use wellbeing_gui::policies::PoliciesViewModel;
use wellbeing_gui::policies::data::{PoliciesRepo, spawn_policies_flow};
use wellbeing_gui::reports::ReportsViewModel;
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

    // BusManager — dual-bus connections with daemon resolution
    let bus = BusManager::connect().await;
    let initial_status = bus.status().await;
    let daemon_available = Arc::new(RwLock::new(initial_status.is_connected()));
    let connection_status = Arc::new(RwLock::new(initial_status));

    // Daemon presence broadcast — instant (dis)appearance detection
    let presence_rx =
        dbus::spawn_daemon_presence_broadcast(&bus.system_connection(), &bus.session_connection());

    // Repositories (timeout-guarded D-Bus access)
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

    // Daemon presence → AppState connection status
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

    // Broadcast channels for per-flow manual refresh triggers
    let (dash_refresh_tx, dash_refresh_rx) = broadcast::channel::<()>(16);
    let (pol_refresh_tx, pol_refresh_rx) = broadcast::channel::<()>(16);
    let (rep_refresh_tx, rep_refresh_rx) = broadcast::channel::<()>(16);

    // 3 watch channels — latest-value, 1 slot each, never block
    let (dash_vm_tx, dash_vm_rx) = watch::channel(None::<DashboardViewModel>);
    let (pol_vm_tx, pol_vm_rx) = watch::channel(None::<PoliciesViewModel>);
    let (rep_vm_tx, rep_vm_rx) = watch::channel(None::<ReportsViewModel>);

    // Spawn background flows (one per screen)
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
                a.set_pol_refresh_tx(pol_refresh_tx.clone());
                a
            });

            // ── 3 independent per-screen ViewModel receivers ──────────────
            // Each watches its own watch::channel and applies the latest VM
            // directly to the App entity.  No merge task, no bundling.
            // .detach() means the tasks self-clean when the window closes.

            // Dashboard receiver — skip no-op updates to prevent
            // unnecessary UI churn and timer re-arming.
            let entity = app_view.clone();
            let mut rx = dash_vm_rx.clone();
            let mut prev_vm: Option<DashboardViewModel> = None;
            cx.spawn(async move |cx| {
                while tokio::task::unconstrained(rx.changed()).await.is_ok() {
                    let vm = rx.borrow_and_update().clone();
                    if prev_vm.as_ref() != vm.as_ref() {
                        prev_vm = vm;
                        entity.update(cx, |app, cx| {
                            app.set_dashboard_vm(Some(prev_vm.clone().unwrap()));
                            app.sync_list_delegates(cx);
                            cx.notify();
                        });
                    }
                }
            })
            .detach();

            // Policies receiver — skip no-op updates.
            let entity = app_view.clone();
            let mut rx = pol_vm_rx.clone();
            let mut prev_vm: Option<PoliciesViewModel> = None;
            cx.spawn(async move |cx| {
                while tokio::task::unconstrained(rx.changed()).await.is_ok() {
                    let vm = rx.borrow_and_update().clone();
                    if prev_vm.as_ref() != vm.as_ref() {
                        prev_vm = vm;
                        entity.update(cx, |app, cx| {
                            app.set_policies_vm(Some(prev_vm.clone().unwrap()));
                            app.sync_list_delegates(cx);
                            cx.notify();
                        });
                    }
                }
            })
            .detach();

            // Reports receiver — skip no-op updates.
            let entity = app_view.clone();
            let mut rx = rep_vm_rx.clone();
            let mut prev_vm: Option<ReportsViewModel> = None;
            cx.spawn(async move |cx| {
                while tokio::task::unconstrained(rx.changed()).await.is_ok() {
                    let vm = rx.borrow_and_update().clone();
                    if prev_vm.as_ref() != vm.as_ref() {
                        prev_vm = vm;
                        entity.update(cx, |app, cx| {
                            app.set_reports_vm(Some(prev_vm.clone().unwrap()));
                            app.sync_list_delegates(cx);
                            cx.notify();
                        });
                    }
                }
            })
            .detach();

            // ── Initial refresh triggers (AFTER receivers registered) ──
            // Send AFTER receivers are spawned so the initial VM is not missed.
            // watch receivers catch up via borrow() if the flow sends before
            // this point, but sending after ensures no race.
            let _ = dash_refresh_tx.send(());
            let _ = pol_refresh_tx.send(());
            let _ = rep_refresh_tx.send(());

            cx.new(|cx| Root::new(app_view, window, cx).bg(cx.theme().background))
        })
        .expect("failed to open window");
    });
}
