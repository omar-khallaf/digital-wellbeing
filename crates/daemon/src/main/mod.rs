//! wellbeing-daemon — Digital Wellbeing system service.
//! Starts the D-Bus server, platform layer, and background actors.

mod watchdog;
mod wiring;

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Context, Result};
use futures::StreamExt;
use tokio::sync::{RwLock, mpsc, oneshot};
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};
use wellbeing_core::BlockedAppsChangedSignal;
use wellbeing_core::SystemClock;
use wellbeing_core::dbus_constants::{
    BLOCKED_APPS_CHANGED_SIGNAL, DAEMON_BUS_NAME, DAEMON_INTERFACE, DAEMON_OBJECT_PATH,
    DAILY_USAGE_CHANGED_SIGNAL,
};
use wellbeing_daemon::{
    bus_resolution::{self, BusMode, DaemonMode},
    categorization::data::CategorizationRepo,
    dbus::DaemonInterface,
    dbus::DaemonInterfaceConfig,
    policy::data::PolicyRepo,
    reports::data::ReportsRepo,
    signal::DaemonSignal,
};

use wellbeing_daemon::blocking::InternalEvent;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "wellbeing_daemon=info,warn".into()),
        )
        .init();

    let mode = bus_resolution::resolve_daemon_mode();
    info!(?mode, "daemon starting");

    let bus = bus_resolution::resolve_bus(&mode);

    let db_path = match &mode {
        DaemonMode::System { db_path } | DaemonMode::Session { db_path, .. } => {
            if let Some(parent) = db_path.parent() {
                tokio::fs::create_dir_all(parent)
                    .await
                    .context("failed to create DB directory")?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    if let Err(e) =
                        tokio::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))
                            .await
                    {
                        warn!(
                            error = %e,
                            "Failed to set DB directory permissions — using default umask"
                        );
                    }
                }
            }
            db_path.clone()
        }
    };

    let pool = wellbeing_daemon::store::StoreBuilder::new(db_path)
        .build()
        .await
        .context("failed to build database pool")?;
    info!("database ready");

    wiring::spawn_prune_loop(pool.clone());

    let (platform, event_stream) = wellbeing_daemon::platform::linux::LinuxPlatformBuilder::new()
        .build(bus)
        .await
        .context("failed to build platform")?;
    let registry = platform.registry();
    let platform = Arc::new(platform);
    info!("platform layer ready");

    let (signal_tx, mut signal_rx) =
        mpsc::unbounded_channel::<wellbeing_daemon::signal::DaemonSignal>();

    let (enforcer_tx, enforcer_rx) =
        mpsc::channel::<wellbeing_daemon::platform::PlatformEvent>(256);

    tokio::spawn(async move {
        let mut stream = event_stream;
        while let Some(event) = stream.next().await {
            if enforcer_tx.send(event).await.is_err() {
                info!("event fan-out: enforcer receiver dropped");
                break;
            }
        }
        info!("event fan-out: platform event stream ended");
    });

    let blocked_apps = Arc::new(RwLock::new(HashMap::new()));

    let shutdown_token = CancellationToken::new();

    let (mut enforcer, internal_rx) = wellbeing_daemon::blocking::EnforcerActor::new(
        pool.clone(),
        platform.clone(),
        registry.clone(),
        SystemClock,
        signal_tx.clone(),
        blocked_apps.clone(),
    );

    // Clone handles BEFORE moving enforcer into the actor task
    let flush_tx = enforcer.flush_handle();
    let policy_tx = enforcer.policy_mutation_handle();

    wiring::spawn_minute_ticker(flush_tx.clone(), shutdown_token.clone());

    tokio::spawn(async move {
        enforcer.run(enforcer_rx, internal_rx).await;
        info!("enforcer actor finished");
    });
    info!("enforcer actor ready");

    let conn = match bus {
        BusMode::System => zbus::Connection::system()
            .await
            .context("failed to connect to system bus")?,
        BusMode::Session => zbus::Connection::session()
            .await
            .context("failed to connect to session bus")?,
    };

    // Shared state for the live serving connection + its unique name.
    // The watchdog updates this on re-acquisition; the signal task reads it
    // on every emit so transient socket loss does not permanently break the daemon.
    let serving_state = Arc::new(RwLock::new(None::<(zbus::Connection, String)>));

    // Clone recovery args before the initial interface consumes them
    let recovery_pool = pool.clone();
    let recovery_registry = registry.clone();
    let recovery_event_tx = platform.event_tx().clone();
    let recovery_policy_tx = policy_tx.clone();
    let recovery_blocked_apps = blocked_apps.clone();

    let policy_repo = PolicyRepo::new(pool.clone());
    let categorization_repo = CategorizationRepo::new(pool.clone());
    let reports_repo = ReportsRepo::new(pool.clone());

    // Register object server BEFORE requesting the name to avoid zbus warnings
    // about method calls arriving before interfaces exist.
    let interface = DaemonInterface::new(DaemonInterfaceConfig {
        policy_repo,
        categorization_repo,
        reports_repo,
        registry,
        event_tx: platform.event_tx(),
        clock: Box::new(SystemClock),
        blocked_apps,
        tokio_handle: tokio::runtime::Handle::current(),
        policy_tx,
    });
    conn.object_server()
        .at(DAEMON_OBJECT_PATH, interface)
        .await
        .context("failed to register D-Bus object")?;

    conn.request_name(DAEMON_BUS_NAME)
        .await
        .context("failed to acquire D-Bus name")?;
    info!("D-Bus server ready on {bus:?} bus");

    let our_unique_name = conn
        .unique_name()
        .map(|n| n.to_string())
        .unwrap_or_default();
    *serving_state.write().await = Some((conn.clone(), our_unique_name));

    let watchdog_handle = watchdog::spawn(
        bus,
        recovery_pool,
        recovery_registry,
        recovery_event_tx,
        recovery_policy_tx,
        recovery_blocked_apps,
        serving_state.clone(),
        shutdown_token.clone(),
    );

    let dbus_state = serving_state.clone();
    let signal_shutdown = shutdown_token.clone();
    let signal_handle = tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = signal_shutdown.cancelled() => {
                    info!("signal task: shutting down");
                    break;
                }
                signal = signal_rx.recv() => {
                    let Some(signal) = signal else { break };
                    let conn = dbus_state
                        .read()
                        .await
                        .as_ref()
                        .map(|(c, _)| c.clone());
                    let Some(conn) = conn else { continue };
                    emit_signal(&conn, signal).await;
                }
            }
        }
    });

    info!("digital-wellbeing daemon started");

    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .expect("failed to install SIGTERM handler");
    let mut int = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())
        .expect("failed to install SIGINT handler");

    tokio::select! {
        _ = shutdown_token.cancelled() => {}
        _ = term.recv() => {}
        _ = int.recv() => {}
    }
    info!("shutting down");

    // Take a logind shutdown-inhibit lock so logind waits while we flush
    let _inhibit = match wellbeing_daemon::logind::take_shutdown_inhibit(
        "Flushing focus events to database before exit",
    )
    .await
    {
        Ok(lock) => {
            info!("logind shutdown-inhibit acquired");
            Some(lock)
        }
        Err(e) => {
            warn!(error = %e, "logind shutdown-inhibit unavailable — proceeding without it");
            None
        }
    };

    let (done_tx, done_rx) = oneshot::channel();
    if flush_tx
        .send(InternalEvent::Flush(Some(done_tx)))
        .await
        .is_ok()
    {
        let _ = done_rx.await;
    }

    // Sever D-Bus serving BEFORE cancelling background tasks.
    // Closing the connection first drops the object server, preventing new
    // D-Bus method calls from arriving while the pool/runtime unwinds.
    // Without this, clients can dispatch queries during shutdown that fail
    // with "task was cancelled" errors from spawn_blocking.
    *serving_state.write().await = None;
    drop(conn);

    shutdown_token.cancel();
    let _ = watchdog_handle.await;
    let _ = tokio::time::timeout(tokio::time::Duration::from_secs(2), signal_handle).await;

    Ok(())
}

async fn emit_signal(conn: &zbus::Connection, signal: DaemonSignal) {
    match signal {
        DaemonSignal::BlockedAppsChanged {
            uid,
            app_class,
            blocked,
            reason,
        } => {
            let payload = BlockedAppsChangedSignal {
                uid,
                app_class,
                blocked,
                reason,
            };
            if let Err(e) = conn
                .emit_signal(
                    None::<&str>,
                    DAEMON_OBJECT_PATH,
                    DAEMON_INTERFACE,
                    BLOCKED_APPS_CHANGED_SIGNAL,
                    &payload,
                )
                .await
            {
                error!(error = %e, "Failed to emit block_state_changed");
            }
        }
        DaemonSignal::DailyUsageChanged { uid } => {
            if let Err(e) = conn
                .emit_signal(
                    None::<&str>,
                    DAEMON_OBJECT_PATH,
                    DAEMON_INTERFACE,
                    DAILY_USAGE_CHANGED_SIGNAL,
                    &uid,
                )
                .await
            {
                error!(error = %e, "Failed to emit daily_usage_changed");
            }
        }
    }
}
