//! Integration with systemd-logind for shutdown-inhibit support.
//!
//! When the daemon receives a termination signal it takes a temporary
//! logind shutdown-inhibit lock via the
//! `org.freedesktop.login1.Manager.Inhibit` D-Bus method. This prevents
//! the system from completing its shutdown sequence until the daemon has
//! flushed its event buffer to the database.
//!
//! The lock is held by keeping the returned file descriptor open. Dropping
//! [`InhibitLock`] closes the fd and releases the inhibitor.

use std::os::unix::io::{AsFd, BorrowedFd, OwnedFd};

use tracing::warn;

/// A handle that holds a logind shutdown-inhibit lock.
///
/// While this object lives, logind delays the shutdown sequence (up to
/// `InhibitDelayMaxSec` in logind.conf, default 5 s). Dropping it closes
/// the underlying file descriptor, releasing the lock.
#[must_use]
pub struct InhibitLock {
    fd: zvariant::OwnedFd,
}

impl AsFd for InhibitLock {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }
}

impl From<InhibitLock> for OwnedFd {
    fn from(lock: InhibitLock) -> Self {
        lock.fd.into()
    }
}

/// Acquire a logind shutdown-inhibit lock with `mode = "delay"`.
///
/// The lock is released when the returned [`InhibitLock`] is dropped.
/// If logind is not available or the D-Bus call fails, an error is returned —
/// callers should treat this as best-effort and proceed without the lock.
///
/// # Arguments
///
/// * `reason` – Human-readable reason string passed to logind as `why`.
///   Example: `"Flushing focus events to database before exit"`.
pub async fn take_shutdown_inhibit(reason: &str) -> anyhow::Result<InhibitLock> {
    let conn = match zbus::Connection::system().await {
        Ok(c) => c,
        Err(e) => {
            warn!(error = %e, "logind: cannot connect to system bus — inhibit unavailable");
            return Err(e.into());
        }
    };

    let result = conn
        .call_method(
            Some("org.freedesktop.login1"),
            "/org/freedesktop/login1",
            Some("org.freedesktop.login1.Manager"),
            "Inhibit",
            &("shutdown", "Digital Wellbeing Daemon", reason, "delay"),
        )
        .await
        .map_err(|e| {
            warn!(error = %e, "logind: Inhibit() call failed — proceeding without lock");
            e
        })?;

    let fd: zvariant::OwnedFd = result.body().deserialize_unchecked().map_err(|e| {
        warn!(error = %e, "logind: failed to extract fd from Inhibit reply");
        e
    })?;

    Ok(InhibitLock { fd })
}
