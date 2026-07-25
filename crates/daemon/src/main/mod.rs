//! wellbeing-daemon — Digital Wellbeing system service.
//! Starts the D-Bus server, platform layer, and background actors.

mod wiring;

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use anyhow::{Context, Result};
use futures::StreamExt;
use tokio::sync::{RwLock, mpsc, oneshot};
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};
use wellbeing_core::SystemClock;
use wellbeing_core::dbus_constants::{
    BLOCKED_APPS_CHANGED_SIGNAL, DAEMON_BUS_NAME, DAEMON_INTERFACE, DAEMON_OBJECT_PATH,
    DAILY_USAGE_CHANGED_SIGNAL,
};
use wellbeing_daemon::{
    bus_resolution::{self, BusMode, DaemonMode},
    dbus::DaemonInterface,
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
                // Directory contains sensitive usage data.
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    if let Err(e) =
                        tokio::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))
                            .await
                    {
                        tracing::warn!(
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

    // Background prune loop for old usage records
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

    // Create shutdown token early so the minute-ticker and signal/watchdog
    // tasks can all reference it.
    let shutdown_token = CancellationToken::new();

    let (mut enforcer, internal_rx) = wellbeing_daemon::blocking::EnforcerActor::new(
        pool.clone(),
        platform.clone(),
        SystemClock,
        signal_tx.clone(),
        blocked_apps.clone(),
    );

    // Clone flush handle BEFORE moving enforcer into the actor task.
    // `flush_tx` stays in main's scope; only clones are moved into spawns.
    let flush_tx = enforcer.flush_handle();

    // Wall-clock aligned minute-ticker
    wiring::spawn_minute_ticker(flush_tx.clone(), shutdown_token.clone());

    tokio::spawn(async move {
        enforcer.run(enforcer_rx, internal_rx).await;
        info!("enforcer actor finished");
    });
    info!("enforcer actor ready");

    // Start screen-lock watcher BEFORE power-state watcher so the
    // lock-state flag is available when the resume handler runs.
    let (screen_lock_rx, screen_is_locked) =
        wellbeing_daemon::platform::linux::ScreenLockWatcher::watch()
            .await
            .context("failed to start ScreenLockWatcher")?;

    let power_rx = wellbeing_daemon::platform::linux::PowerStateWatcher::watch()
        .await
        .context("failed to start PowerStateWatcher")?;
    let power_flush_tx = flush_tx.clone();
    let power_tx = platform.event_tx();
    let shutdown_tx = power_tx.clone();
    let resume_is_locked = screen_is_locked.clone();
    let power_registry = registry.clone();
    tokio::spawn(async move {
        use tokio_stream::wrappers::UnboundedReceiverStream;
        use wellbeing_daemon::platform::PlatformEvent;
        use wellbeing_daemon::platform::linux::PowerEvent;
        use wellbeing_daemon::platform::linux::inhibit_shutdown;

        let mut power_stream = UnboundedReceiverStream::new(power_rx);
        while let Some(event) = power_stream.next().await {
            // Create a logind delay inhibitor for sleep/shutdown/logout.
            // The OwnedFd is kept alive until the flush ack arrives,
            // then dropped at the end of this iteration to release logind.
            let _inhibit_fd = match event {
                PowerEvent::Slept | PowerEvent::ShutDown | PowerEvent::LoggedOut => {
                    match inhibit_shutdown().await {
                        Ok(fd) => Some(fd),
                        Err(e) => {
                            error!(error = %e, "failed to inhibit logind, proceeding without delay");
                            None
                        }
                    }
                }
                _ => None,
            };

            let (platform_event, is_resume) = match event {
                PowerEvent::Slept => (PlatformEvent::Slept, false),
                PowerEvent::ShutDown => (PlatformEvent::ShutDown, false),
                PowerEvent::ResumedSystem => (PlatformEvent::ResumedSystem, true),
                PowerEvent::LoggedOut => (PlatformEvent::LoggedOut, false),
            };

            let is_power_change = matches!(
                platform_event,
                PlatformEvent::Slept | PlatformEvent::ShutDown | PlatformEvent::LoggedOut
            );

            if is_power_change {
                // 1. Send the close event to enforcer so it drains current_focus
                if power_tx.send(platform_event).is_err() {
                    break;
                }
                // 2. Yield so the enforcer buffers the event before we flush
                tokio::time::sleep(Duration::from_millis(50)).await;
                // 3. Flush with ack — persists the close event to the database
                let (ack_tx, ack_rx) = oneshot::channel();
                if power_flush_tx
                    .send(InternalEvent::Flush(Some(ack_tx)))
                    .await
                    .is_ok()
                {
                    let _ = ack_rx.await;
                }
                // 4. inhibit_fd dropped here → logind proceeds with the state change
            } else if is_resume {
                if power_tx.send(platform_event).is_err() {
                    break;
                }
                if !resume_is_locked.load(Ordering::Acquire) {
                    // Screen was unlocked during sleep — resync focus by
                    // querying the plugin's CurrentFocus property.
                    let reg = power_registry.read().await;
                    for uid in reg.registered_uids() {
                        if let Some(event) = reg.current_focus_for_uid(uid).await
                            && power_tx.send(event).is_err()
                        {
                            break;
                        }
                    }
                }
                // If screen is locked, resync happens when the user unlocks.
            } else {
                if power_tx.send(platform_event).is_err() {
                    break;
                }
            }
        }
    });

    // Spawn screen-lock watcher — listens for lock/unlock events and
    // re-synchronizes focus tracking on unlock.
    wiring::spawn_screen_lock_watcher(platform.event_tx(), registry.clone(), screen_lock_rx);

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
    let serving_state = Arc::new(RwLock::new(Option::<(zbus::Connection, String)>::None));

    // Clone recovery args before the initial interface consumes them.
    let recovery_pool = pool.clone();
    let recovery_registry = registry.clone();
    let recovery_event_tx = platform.event_tx().clone();
    let recovery_blocked_apps = blocked_apps.clone();
    // Build interface before touching the connection so we can register
    // the object server atomically with the name request.
    let interface = DaemonInterface::new(
        pool,
        registry,
        platform.event_tx(),
        Box::new(SystemClock),
        blocked_apps,
        tokio::runtime::Handle::current(),
    );

    // Register object server BEFORE requesting the name. This eliminates
    // the zbus warning about method calls arriving before interfaces exist.
    conn.object_server()
        .at(DAEMON_OBJECT_PATH, interface)
        .await
        .context("failed to register D-Bus object")?;

    // Acquire well-known D-Bus name. Fail fast if we can't own it —
    // the daemon is unreachable without it.
    conn.request_name(DAEMON_BUS_NAME)
        .await
        .context("failed to acquire D-Bus name")?;
    info!("D-Bus server ready on {bus:?} bus");

    let our_unique_name = conn
        .unique_name()
        .map(|n| n.to_string())
        .unwrap_or_default();
    *serving_state.write().await = Some((conn.clone(), our_unique_name.clone()));

    // Watchdog: monitor for D-Bus name loss. Uses a separate probe connection
    // so I/O errors in the watch stream cannot corrupt the daemon's serving
    // connection. On re-acquisition, builds a FRESH connection + interface so
    // a dead socket cannot be silently reused.
    let watchdog_token = shutdown_token.clone();
    let watchdog_name = DAEMON_BUS_NAME.to_string();
    let bus_mode = bus;
    let recovery_state = serving_state.clone();
    let watchdog_token_for_task = watchdog_token.clone();
    let watchdog_handle = tokio::spawn(async move {
        #[zbus::proxy(
            default_service = "org.freedesktop.DBus",
            default_path = "/org/freedesktop/DBus",
            interface = "org.freedesktop.DBus"
        )]
        trait DBusFdo {
            #[zbus(signal)]
            fn name_owner_changed(
                &self,
                name: String,
                old_owner: String,
                new_owner: String,
            ) -> zbus::Result<()>;
            #[zbus(signal)]
            fn name_lost(&self, name: String) -> zbus::Result<()>;
        }

        loop {
            let watch_conn = match bus_mode {
                BusMode::System => match zbus::Connection::system().await {
                    Ok(c) => c,
                    Err(e) => {
                        error!(%e, "watchdog: failed to connect to system bus");
                        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                        continue;
                    }
                },
                BusMode::Session => match zbus::Connection::session().await {
                    Ok(c) => c,
                    Err(e) => {
                        error!(%e, "watchdog: failed to connect to session bus");
                        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                        continue;
                    }
                },
            };

            let proxy = match DBusFdoProxy::new(&watch_conn).await {
                Ok(p) => p,
                Err(e) => {
                    error!(%e, "watchdog: failed to create DBus proxy");
                    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                    continue;
                }
            };

            let mut stream = match proxy.receive_name_owner_changed().await {
                Ok(s) => s,
                Err(e) => {
                    error!(%e, "watchdog: failed to subscribe to NameOwnerChanged signal");
                    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                    continue;
                }
            };

            loop {
                tokio::select! {
                    msg = stream.next() => {
                        match msg {
                            Some(msg) => {
                                match msg.args() {
                                    Ok(args) => {
                                        if args.name == watchdog_name {
                                            // Read the CURRENT unique name from shared state so we
                                            // match after every re-acquisition, not just the first.
                                            let current_name = recovery_state
                                                .read()
                                                .await
                                                .as_ref()
                                                .map(|(_, name)| name.clone());
                                            if let Some(our_name) = current_name
                                                && args.old_owner == our_name {
                                                    error!(name = ?args.name, old_owner = ?args.old_owner, new_owner = ?args.new_owner, "D-Bus name lost, attempting re-acquisition");
                                                    let mut retries = 0u32;
                                                    loop {
                                                        if watchdog_token_for_task.is_cancelled() {
                                                            info!("watchdog: re-acquisition cancelled");
                                                            return;
                                                        }

                                                        let fresh_conn = match bus_mode {
                                                            BusMode::System => match zbus::Connection::system().await {
                                                                Ok(c) => c,
                                                                Err(e) => {
                                                                    error!(%e, "watchdog: failed to create fresh system bus connection");
                                                                    retries += 1;
                                                                    if retries >= 5 {
                                                                        error!("max retries exceeded, shutting down");
                                                                        watchdog_token_for_task.cancel();
                                                                        return;
                                                                    }
                                                                    tokio::time::sleep(
                                                                        tokio::time::Duration::from_millis(500 * retries as u64),
                                                                    )
                                                                    .await;
                                                                    continue;
                                                                }
                                                            },
                                                            BusMode::Session => match zbus::Connection::session().await {
                                                                Ok(c) => c,
                                                                Err(e) => {
                                                                    error!(%e, "watchdog: failed to create fresh session bus connection");
                                                                    retries += 1;
                                                                    if retries >= 5 {
                                                                        error!("max retries exceeded, shutting down");
                                                                        watchdog_token_for_task.cancel();
                                                                        return;
                                                                    }
                                                                    tokio::time::sleep(
                                                                        tokio::time::Duration::from_millis(500 * retries as u64),
                                                                    )
                                                                    .await;
                                                                    continue;
                                                                }
                                                            },
                                                        };

                                                        let fresh_interface = DaemonInterface::new(
                                                            recovery_pool.clone(),
                                                            recovery_registry.clone(),
                                                            recovery_event_tx.clone(),
                                                            Box::new(SystemClock),
                                                            recovery_blocked_apps.clone(),
                                                            tokio::runtime::Handle::current(),
                                                        );

                                                        if let Err(e) = fresh_conn
                                                            .object_server()
                                                            .at(DAEMON_OBJECT_PATH, fresh_interface)
                                                            .await
                                                        {
                                                            error!(%e, "watchdog: failed to register object server on fresh connection");
                                                            retries += 1;
                                                            if retries >= 5 {
                                                                error!("max retries exceeded, shutting down");
                                                                watchdog_token_for_task.cancel();
                                                                return;
                                                            }
                                                            tokio::time::sleep(
                                                                tokio::time::Duration::from_millis(500 * retries as u64),
                                                            )
                                                            .await;
                                                            continue;
                                                        }

                                                        match fresh_conn
                                                            .request_name(DAEMON_BUS_NAME)
                                                            .await
                                                        {
                                                            Ok(_) => {
                                                                let new_unique = fresh_conn.unique_name().map(|n| n.to_string()).unwrap_or_default();
                                                                info!(unique_name = ?new_unique, "D-Bus name re-acquired on fresh connection");

                                                                // Stabilization delay: the bus may still be cleaning up
                                                                // the old connection's state. Waiting briefly reduces the
                                                                // chance of immediate re-loss from external interference.
                                                                tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

                                                                let still_ok = fresh_conn.unique_name().is_some();
                                                                if still_ok {
                                                                    info!(unique_name = ?new_unique, "D-Bus connection stable after re-acquisition");
                                                                    *recovery_state.write().await = Some((fresh_conn, new_unique));
                                                                    break;
                                                                } else {
                                                                    error!("fresh connection lost unique name during stabilization, retrying");
                                                                    retries += 1;
                                                                    if retries >= 5 {
                                                                        error!("max retries exceeded, shutting down");
                                                                        watchdog_token_for_task.cancel();
                                                                        return;
                                                                    }
                                                                    tokio::time::sleep(
                                                                        tokio::time::Duration::from_millis(500 * retries as u64),
                                                                    )
                                                                    .await;
                                                                    continue;
                                                                }
                                                            }
                                                            Err(e) => {
                                                                error!(%e, "watchdog: failed to re-acquire D-Bus name");
                                                                retries += 1;
                                                                if retries >= 5 {
                                                                    error!("max retries exceeded, shutting down");
                                                                    watchdog_token_for_task.cancel();
                                                                    return;
                                                                }
                                                                tokio::time::sleep(
                                                                    tokio::time::Duration::from_millis(500 * retries as u64),
                                                                )
                                                                .await;
                                                            }
                                                        }
                                                    }
                                                }
                                        }
                                    }
                                    Err(e) => {
                                        error!(%e, "watchdog: failed to parse NameOwnerChanged args");
                                    }
                                }
                            }
                            None => {
                                break;
                            }
                        }
                    }
                    _ = watchdog_token_for_task.cancelled() => {
                        info!("watchdog: shutting down");
                        return;
                    }
                }
            }

            warn!("NameOwnerChanged stream ended, restarting watch in 1s");
            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
        }
    });

    let dbus_conn = serving_state.clone();
    let signal_shutdown = shutdown_token.clone();
    let signal_handle = tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = signal_shutdown.cancelled() => {
                    info!("signal task: shutting down");
                    break;
                }
                signal = signal_rx.recv() => {
                    match signal {
                        Some(signal) => {
                            let conn = dbus_conn
                                .read()
                                .await
                                .as_ref()
                                .map(|(c, _)| c.clone());
                            let Some(conn) = conn else { continue };
                            match signal {
                                DaemonSignal::BlockedAppsChanged {
                                    uid,
                                    app_id,
                                    blocked,
                                    reason,
                                } => {
                                    let app_id_str = app_id.as_ref().to_string();
                                    if let Err(e) = conn
                                        .emit_signal(
                                            None::<&str>,
                                            DAEMON_OBJECT_PATH,
                                            DAEMON_INTERFACE,
                                            BLOCKED_APPS_CHANGED_SIGNAL,
                                            &(uid, app_id_str, blocked, reason),
                                        )
                                        .await
                                    {
                                        tracing::error!(error = %e, "Failed to emit block_state_changed");
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
                                        tracing::error!(error = %e, "Failed to emit daily_usage_changed");
                                    }
                                }
                            }
                        }
                        None => break,
                    }
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

    // Send ShutDown BEFORE flushing so the enforcer buffers close events
    // and the following flush persists them to the database.
    let _ = shutdown_tx.send(wellbeing_daemon::platform::PlatformEvent::ShutDown);
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // Flush remaining events (including ShutDown buffer) to DB
    let (done_tx, done_rx) = oneshot::channel();
    if flush_tx
        .send(InternalEvent::Flush(Some(done_tx)))
        .await
        .is_ok()
    {
        let _ = done_rx.await;
    }

    watchdog_token.cancel();

    let _ = watchdog_handle.await;
    let _ = tokio::time::timeout(tokio::time::Duration::from_secs(2), signal_handle).await;

    drop(serving_state);

    Ok(())
}
