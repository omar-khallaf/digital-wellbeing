//! Query handler helpers — shared error-mapping and common patterns.
//!
//! D-Bus interface methods that query the database share a nearly
//! identical `.map_err(|e| { tracing::error!(); fdo::Error::Failed(...) })`
//! pattern. This module provides a single `map_err` helper to DRY it up.

/// Map a database / domain error to a zbus D-Bus error after logging.
///
/// # Example
///
/// ```ignore
/// .map_err(|e| query_handlers::map_err(e, "insert failed"))
/// ```
pub(crate) fn map_err<E: std::fmt::Display>(e: E, msg: &'static str) -> zbus::fdo::Error {
    tracing::error!(error = %e, "{msg}");
    zbus::fdo::Error::Failed("internal error".into())
}
