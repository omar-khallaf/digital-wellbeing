//! Core D-Bus utilities — authentication and authorization helpers.

use wellbeing_core::Uid;
use zbus::fdo;

pub(crate) async fn authenticate(
    conn: &zbus::Connection,
    header: zbus::message::Header<'_>,
) -> Result<Uid, zbus::fdo::Error> {
    require_sender(&header)?;
    let creds = conn.peer_creds().await.map_err(|e| {
        tracing::error!(error = %e, "failed to read peer credentials");
        fdo::Error::Failed("internal error".into())
    })?;
    creds.unix_user_id().map(Uid).ok_or_else(|| {
        tracing::error!("no unix uid in peer credentials");
        fdo::Error::Failed("internal error".into())
    })
}

pub(crate) fn require_sender(header: &zbus::message::Header<'_>) -> Result<(), fdo::Error> {
    header.sender().map(|_| ()).ok_or_else(|| {
        tracing::error!("no sender in message header");
        fdo::Error::Failed("internal error".into())
    })
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
    fn missing_sender_fails() {
        let msg =
            zbus::message::Message::method_call("/org/wellbeing/v1/Controller", "ListPolicies")
                .unwrap()
                .build(&())
                .unwrap();
        let header = msg.header();
        assert!(header.sender().is_none());
        let err = require_sender(&header).unwrap_err();
        assert!(matches!(err, fdo::Error::Failed(_)));
    }
}
