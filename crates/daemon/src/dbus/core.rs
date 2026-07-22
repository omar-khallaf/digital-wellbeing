//! Core D-Bus utilities — authentication and authorization helpers.

use zbus::fdo;

/// Authenticate a D-Bus caller by extracting their Unix UID from
/// the message header's connection credentials.
pub(crate) async fn authenticate(
    conn: &zbus::Connection,
    header: zbus::message::Header<'_>,
) -> Result<u32, zbus::fdo::Error> {
    let sender = header.sender().ok_or_else(|| {
        tracing::error!("no sender in message header");
        fdo::Error::Failed("internal error".into())
    })?;

    let dbus_proxy = fdo::DBusProxy::new(conn).await.map_err(|e| {
        tracing::error!(error = %e, "failed to create DBusProxy");
        fdo::Error::Failed("internal error".into())
    })?;

    let creds = dbus_proxy
        .get_connection_credentials(sender.clone().into())
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "failed to get connection credentials");
            fdo::Error::Failed("internal error".into())
        })?;

    creds.unix_user_id().ok_or_else(|| {
        tracing::error!("no unix uid in caller credentials");
        fdo::Error::Failed("internal error".into())
    })
}

/// Resolve the effective UID: root (uid=0) can act on behalf of any user,
/// non-root users are always resolved to their own UID.
pub(crate) fn resolve_uid(caller: u32, target: u32) -> u32 {
    if caller == 0 { target } else { caller }
}
