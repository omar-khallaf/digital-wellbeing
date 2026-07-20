//! wellbeing-daemon — Digital Wellbeing system service.
//! Starts the D-Bus server, platform layer, and background actors.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use futures::StreamExt;
use tokio::sync::{RwLock, mpsc};
use tracing::{error, info};
use wellbeing_core::{SystemClock, Uid};

#[derive(Debug, Clone, Copy)]
enum BusMode {
    System,
    Session,
}

#[derive(Debug, Clone)]
enum DaemonMode {
    System { db_path: PathBuf },
    Session { db_path: PathBuf, _uid: Uid },
}

fn resolve_daemon_mode() -> DaemonMode {
    let euid = nix::unistd::Uid::effective();
    if euid.is_root() {
        DaemonMode::System {
            db_path: PathBuf::from("/var/lib/digital-wellbeing/db.sqlite"),
        }
    } else {
        let xdg_data = std::env::var("XDG_DATA_HOME").unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
            format!("{home}/.local/share")
        });
        DaemonMode::Session {
            db_path: PathBuf::from(xdg_data).join("digital-wellbeing/db.sqlite"),
            _uid: Uid(euid.as_raw()),
        }
    }
}

fn resolve_bus(mode: &DaemonMode) -> BusMode {
    match mode {
        DaemonMode::System { .. } => BusMode::System,
        DaemonMode::Session { .. } => BusMode::Session,
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "wellbeing_daemon=info,warn".into()),
        )
        .init();

    let mode = resolve_daemon_mode();
    info!(?mode, "daemon starting");

    let db_path = match &mode {
        DaemonMode::System { db_path } | DaemonMode::Session { db_path, .. } => {
            if let Some(parent) = db_path.parent() {
                tokio::fs::create_dir_all(parent)
                    .await
                    .context("failed to create DB directory")?;
            }
            db_path.clone()
        }
    };

    let pool = wellbeing_daemon::store::StoreBuilder::new(db_path)
        .build()
        .await
        .context("failed to build database pool")?;
    info!("database ready");

    let prune_pool = pool.clone();
    tokio::spawn(async move {
        wellbeing_daemon::reports::prune_loop(prune_pool, Box::new(SystemClock)).await;
    });

    let (platform, event_stream) =
        wellbeing_daemon::platform::linux::LinuxPlatformBuilder::new(pool.clone())
            .build()
            .await
            .context("failed to build platform")?;
    let registry = platform.registry();
    let platform = Arc::new(platform);
    info!("platform layer ready");

    let (signal_tx, mut signal_rx) =
        mpsc::unbounded_channel::<wellbeing_daemon::signal::DaemonSignal>();

    let (tracker_tx, tracker_rx) = mpsc::channel::<wellbeing_daemon::platform::PlatformEvent>(256);
    let (enforcer_tx, mut enforcer_rx) =
        mpsc::channel::<wellbeing_daemon::platform::PlatformEvent>(256);

    tokio::spawn(async move {
        let mut stream = event_stream;
        while let Some(event) = stream.next().await {
            if tracker_tx.send(event.clone()).await.is_err() {
                info!("event fan-out: tracker receiver dropped");
                break;
            }
            if enforcer_tx.send(event).await.is_err() {
                info!("event fan-out: enforcer receiver dropped");
                break;
            }
        }
        info!("event fan-out: platform event stream ended");
    });

    let (notifier, _notifier_rx) = wellbeing_daemon::tracking::ReactiveNotifier::new();
    let tracker = wellbeing_daemon::tracking::TrackerActor::new(
        notifier,
        Box::new(SystemClock),
        signal_tx.clone(),
    );
    tokio::spawn(async move {
        tracker.run(tracker_rx).await;
        info!("tracker actor finished");
    });
    info!("tracker actor ready");

    let active_blocks = Arc::new(RwLock::new(HashMap::new()));

    let mut enforcer = wellbeing_daemon::blocking::EnforcerActor::new(
        pool.clone(),
        platform.clone(),
        Box::new(SystemClock),
        signal_tx.clone(),
        active_blocks.clone(),
    );
    tokio::spawn(async move {
        while let Some(event) = enforcer_rx.recv().await {
            enforcer.handle_event(event).await;
        }
        info!("enforcer actor finished");
    });
    info!("enforcer actor ready");

    let power_rx = wellbeing_daemon::platform::linux::PowerStateWatcher::watch(
        pool.clone(),
        Box::new(SystemClock),
    )
    .await
    .context("failed to start PowerStateWatcher")?;
    let power_tx = platform.event_tx();
    let shutdown_tx = power_tx.clone();
    tokio::spawn(async move {
        use futures::StreamExt;
        use tokio_stream::wrappers::UnboundedReceiverStream;
        let mut power_stream = UnboundedReceiverStream::new(power_rx);
        while let Some(event) = power_stream.next().await {
            let platform_event = match event {
                wellbeing_daemon::platform::linux::PowerEvent::Slept => {
                    wellbeing_daemon::platform::PlatformEvent::Slept
                }
                wellbeing_daemon::platform::linux::PowerEvent::ShutDown => {
                    wellbeing_daemon::platform::PlatformEvent::ShutDown
                }
                wellbeing_daemon::platform::linux::PowerEvent::Locked => {
                    wellbeing_daemon::platform::PlatformEvent::Locked
                }
                wellbeing_daemon::platform::linux::PowerEvent::LoggedOut => {
                    wellbeing_daemon::platform::PlatformEvent::LoggedOut
                }
            };
            if power_tx.send(platform_event).is_err() {
                info!("power event channel closed");
                break;
            }
        }
    });

    let bus = resolve_bus(&mode);
    let conn = match bus {
        BusMode::System => zbus::Connection::system()
            .await
            .context("failed to connect to system bus")?,
        BusMode::Session => zbus::Connection::session()
            .await
            .context("failed to connect to session bus")?,
    };

    let dbus_conn = conn.clone();
    tokio::spawn(async move {
        while let Some(signal) = signal_rx.recv().await {
            match signal {
                wellbeing_daemon::signal::DaemonSignal::BlockStateChanged {
                    uid,
                    app_id,
                    blocked,
                    reason,
                } => {
                    let app_id_str = app_id.as_ref().to_string();
                    if let Err(e) = dbus_conn
                        .emit_signal(
                            None::<&str>,
                            "/org/wellbeing/Daemon",
                            "org.wellbeing.v1.Daemon",
                            "BlockStateChanged",
                            &(uid, &app_id_str, blocked, reason),
                        )
                        .await
                    {
                        tracing::error!(error = %e, "Failed to emit block_state_changed");
                    }
                }
                wellbeing_daemon::signal::DaemonSignal::DailyUsageChanged { uid } => {
                    if let Err(e) = dbus_conn
                        .emit_signal(
                            None::<&str>,
                            "/org/wellbeing/Daemon",
                            "org.wellbeing.v1.Daemon",
                            "DailyUsageChanged",
                            &(uid,),
                        )
                        .await
                    {
                        tracing::error!(error = %e, "Failed to emit daily_usage_changed");
                    }
                }
            }
        }
    });

    let interface = wellbeing_daemon::dbus::DaemonInterface::new(
        pool,
        registry,
        platform.event_tx(),
        Box::new(SystemClock),
        active_blocks,
    );
    tokio::spawn(async move {
        if let Err(e) = conn
            .object_server()
            .at("/org/wellbeing/Daemon", interface)
            .await
        {
            error!("failed to register D-Bus object: {e}");
        }
        info!("D-Bus server ready on {bus:?} bus");

        if let Err(e) = conn.request_name("org.wellbeing.v1.Daemon").await {
            error!("failed to request D-Bus name: {e}");
        }
    });

    info!("digital-wellbeing daemon started");

    shutdown_signal().await;
    info!("shutting down");

    let _ = shutdown_tx.send(wellbeing_daemon::platform::PlatformEvent::ShutDown);
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    Ok(())
}

async fn shutdown_signal() {
    use tokio::signal::unix::{SignalKind, signal};

    let mut term = signal(SignalKind::terminate()).expect("failed to install SIGTERM handler");
    let mut int = signal(SignalKind::interrupt()).expect("failed to install SIGINT handler");

    tokio::select! {
        _ = term.recv() => {}
        _ = int.recv() => {}
    }
}
