//! D-Bus signal emission — runs in a background task, re-reads the serving
//! connection on each emit so transient socket loss does not break the daemon.

use tracing::error;
use wellbeing_core::BlockedAppsChangedSignal;
use wellbeing_core::dbus_constants::{
    BLOCKED_APPS_CHANGED_SIGNAL, DAILY_USAGE_CHANGED_SIGNAL, DAEMON_INTERFACE, DAEMON_OBJECT_PATH,
};
use wellbeing_daemon::signal::DaemonSignal;

pub(crate) async fn emit_signal(conn: &zbus::Connection, signal: DaemonSignal) {
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
