use tracing::error;
use wellbeing_core::dbus_constants::{
    APP_BLOCKED_SIGNAL, DAEMON_INTERFACE, DAEMON_OBJECT_PATH, DOMAIN_BLOCKED_SIGNAL,
    POLICY_CHANGED_SIGNAL, USAGE_UPDATED_SIGNAL,
};
use wellbeing_core::{AppBlockedSignal, DomainBlockedSignal, PolicyChangedSignal, UsageUpdatedSignal};
use wellbeing_daemon::signal::DaemonSignal;

pub(crate) async fn emit_signal(conn: &zbus::Connection, signal: DaemonSignal) {
    match signal {
        DaemonSignal::AppBlocked {
            uid,
            app_class,
            blocked,
            reason,
        } => {
            let payload = AppBlockedSignal {
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
                    APP_BLOCKED_SIGNAL,
                    &payload,
                )
                .await
            {
                error!(error = %e, "Failed to emit app_blocked");
            }
        }
        DaemonSignal::DomainBlocked {
            uid,
            domain,
            blocked,
            reason,
        } => {
            let payload = DomainBlockedSignal {
                uid,
                domain,
                blocked,
                reason,
            };
            if let Err(e) = conn
                .emit_signal(
                    None::<&str>,
                    DAEMON_OBJECT_PATH,
                    DAEMON_INTERFACE,
                    DOMAIN_BLOCKED_SIGNAL,
                    &payload,
                )
                .await
            {
                error!(error = %e, "Failed to emit domain_blocked");
            }
        }
        DaemonSignal::PolicyChanged { uid } => {
            let payload = PolicyChangedSignal { uid };
            if let Err(e) = conn
                .emit_signal(
                    None::<&str>,
                    DAEMON_OBJECT_PATH,
                    DAEMON_INTERFACE,
                    POLICY_CHANGED_SIGNAL,
                    &payload,
                )
                .await
            {
                error!(error = %e, "Failed to emit policy_changed");
            }
        }
        DaemonSignal::UsageUpdated { uid } => {
            let payload = UsageUpdatedSignal { uid };
            if let Err(e) = conn
                .emit_signal(
                    None::<&str>,
                    DAEMON_OBJECT_PATH,
                    DAEMON_INTERFACE,
                    USAGE_UPDATED_SIGNAL,
                    &payload,
                )
                .await
            {
                error!(error = %e, "Failed to emit usage_updated");
            }
        }
    }
}
