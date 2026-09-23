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

pub(crate) fn blocked_entries_for(
    blocks: &std::collections::HashMap<
        wellbeing_core::Uid,
        std::collections::HashMap<wellbeing_core::AppClass, wellbeing_core::BlockedAppEntry>,
    >,
    scope: Option<wellbeing_core::Uid>,
) -> Vec<wellbeing_core::BlockedAppEntry> {
    match scope {
        None => blocks
            .values()
            .flat_map(|per_user| per_user.values())
            .cloned()
            .collect(),
        Some(uid) => blocks
            .get(&uid)
            .map(|per_user| per_user.values().cloned().collect())
            .unwrap_or_default(),
    }
}
