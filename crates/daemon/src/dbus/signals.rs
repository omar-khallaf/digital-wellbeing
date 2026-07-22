//! D-Bus signal emission helpers.
//!
//! These functions emit D-Bus signals via the provided connection.
//! Signal forwarding from daemon-internal events is handled in main.rs.

use wellbeing_core::dbus_constants::{DAEMON_INTERFACE, DAEMON_OBJECT_PATH};

/// Emit a PolicyMutated signal.
pub(crate) async fn policy_mutated(conn: &zbus::Connection, uid: u32) -> Result<(), zbus::Error> {
    conn.emit_signal(
        None::<&str>,
        DAEMON_OBJECT_PATH,
        DAEMON_INTERFACE,
        "PolicyMutated",
        &(uid,),
    )
    .await
}
