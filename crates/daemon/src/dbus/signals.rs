use wellbeing_core::Uid;
use wellbeing_core::dbus_constants::{
    DAEMON_INTERFACE, DAEMON_OBJECT_PATH, POLICY_CHANGED_SIGNAL, USAGE_UPDATED_SIGNAL,
};

pub(crate) async fn policy_changed(conn: &zbus::Connection, uid: Uid) -> Result<(), zbus::Error> {
    conn.emit_signal(
        None::<&str>,
        DAEMON_OBJECT_PATH,
        DAEMON_INTERFACE,
        POLICY_CHANGED_SIGNAL,
        &(uid.0,),
    )
    .await
}

#[allow(dead_code)]
pub(crate) async fn usage_updated(conn: &zbus::Connection, uid: Uid) -> Result<(), zbus::Error> {
    conn.emit_signal(
        None::<&str>,
        DAEMON_OBJECT_PATH,
        DAEMON_INTERFACE,
        USAGE_UPDATED_SIGNAL,
        &(uid.0,),
    )
    .await
}
