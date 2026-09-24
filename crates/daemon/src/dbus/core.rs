//! Core D-Bus utilities — authentication and authorization helpers.

use wellbeing_core::Uid;
use zbus::fdo;

pub(crate) async fn authenticate(
    conn: &zbus::Connection,
    header: zbus::message::Header<'_>,
) -> Result<Uid, zbus::fdo::Error> {
    let sender = header
        .sender()
        .ok_or_else(|| {
            tracing::error!("no sender in message header");
            fdo::Error::Failed("internal error".into())
        })?
        .to_owned();
    let proxy = zbus::fdo::DBusProxy::new(conn).await.map_err(|e| {
        tracing::error!(error = %e, "failed to create DBus proxy for auth");
        fdo::Error::Failed("internal error".into())
    })?;
    let uid = proxy
        .get_connection_unix_user(sender.into())
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "GetConnectionUnixUser failed");
            fdo::Error::Failed("internal error".into())
        })?;
    Ok(Uid(uid))
}

pub(crate) fn require_root(caller: Uid) -> Result<(), fdo::Error> {
    if caller.0 == 0 {
        Ok(())
    } else {
        Err(fdo::Error::AccessDenied("access denied".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn require_root_rejects_non_root() {
        let err = require_root(Uid(1000)).unwrap_err();
        assert!(matches!(err, fdo::Error::AccessDenied(_)));
        assert!(require_root(Uid(0)).is_ok());
    }

    #[test]
    fn locally_built_message_has_no_sender() {
        let msg =
            zbus::message::Message::method_call("/org/wellbeing/v1/Controller", "ListPolicies")
                .unwrap()
                .build(&())
                .unwrap();
        let header = msg.header();
        assert!(header.sender().is_none());
    }
}
