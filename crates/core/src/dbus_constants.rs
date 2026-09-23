//! Shared D-Bus constants for the Digital Wellbeing system.
//! Single source of truth for all bus names, object paths, signal names,
//! and property names used across daemon, GUI, and plugin IPC.

// ── Daemon (Controller) interface ────────────────────────────────────────────

pub const DAEMON_INTERFACE: &str = "org.wellbeing.v1.Controller";

pub const DAEMON_OBJECT_PATH: &str = "/org/wellbeing/Controller";

pub const DAEMON_BUS_NAME: &str = "org.wellbeing.v1.Controller";

// ── Plugin (Manager) interface ───────────────────────────────────────────────

/// Well-known D-Bus interface name for the compositor plugin's Manager API.
pub const MANAGER_INTERFACE: &str = "org.wellbeing.v1.Manager";

pub const MANAGER_OBJECT_PATH: &str = "/org/wellbeing/Manager";

// ── Signal names on the Controller interface ─────────────────────────────────

/// Emitted when an app becomes blocked for a user.
pub const APP_BLOCKED_SIGNAL: &str = "AppBlocked";

/// Emitted when a policy is created, updated, or deleted.
pub const POLICY_CHANGED_SIGNAL: &str = "PolicyChanged";

/// Emitted when a domain becomes blocked for a user.
pub const DOMAIN_BLOCKED_SIGNAL: &str = "DomainBlocked";

/// Emitted every minute tick for each tracked user with fresh usage totals.
pub const USAGE_UPDATED_SIGNAL: &str = "UsageUpdated";

// ── Method names on the Controller interface ─────────────────────────────────

/// Returns all blocked apps across users.
pub const GET_BLOCKED_APPS_METHOD: &str = "GetBlockedApps";

/// Returns blocked apps for a single user.
pub const GET_BLOCKED_APPS_FOR_USER_METHOD: &str = "GetBlockedAppsForUser";

/// Registers the compositor bridge so the daemon can push block state.
pub const REGISTER_BRIDGE_METHOD: &str = "RegisterBridge";

// ── Signal names on the Manager interface ────────────────────────────────────

/// Name of the unified `event` signal.
pub const EVENT_SIGNAL: &str = "Event";

// ── Property names ───────────────────────────────────────────────────────────

/// Read-only property on the Manager interface exposing current session state.
pub const CURRENT_SESSION_PROPERTY: &str = "CurrentSession";

// ═════════════════════════════════════════════════════════════════════════════
// Unified event signal.
//
// The `event` signal carries a D-Bus struct with 4 fields:
//   (u:tag, s:app_class, s:title, u:power_tag)
//
// Signature: `(ussu)`

pub const EVENT_STRUCT_SIGNATURE: &str = "(ussu)";

/// Event tag for Focus — a window received focus. Applies to `uid`.
/// Relevant fields: app_class, title
pub const EVENT_TAG_FOCUS: u32 = 0;

/// Event tag for Unfocus — all windows for `uid` lost focus (desktop shown).
/// Relevant fields: uid only.
pub const EVENT_TAG_UNFOCUS: u32 = 1;

/// Event tag for Block — focus changed to a blocked window (overlay shown).
/// Relevant fields: app_class, title, uid.
pub const EVENT_TAG_BLOCK: u32 = 2;

/// Event tag for Idle — user activity stopped for `uid`.
/// Relevant fields: uid only.
pub const EVENT_TAG_IDLE: u32 = 3;

/// Event tag for Resume — user activity resumed for `uid`.
/// Relevant fields: uid only.
pub const EVENT_TAG_RESUME: u32 = 4;

/// Event tag for LogOut — user session `uid` logged out.
/// Relevant fields: uid only.
pub const EVENT_TAG_LOGOUT: u32 = 5;

/// Event tag for PowerEvent — system power-state change affecting `uid`.
/// Relevant fields: uid, power_tag.
pub const EVENT_TAG_POWER: u32 = 6;

/// Event tag for Locked — session locked (screen saver / logind lock).
/// Relevant fields: uid only.
pub const EVENT_TAG_LOCKED: u32 = 7;

// ── Power tags (inner discriminator for EVENT_TAG_POWER) ─────────────────────

pub const EVENT_POWER_SUSPEND: u32 = 0;

pub const EVENT_POWER_HIBERNATE: u32 = 1;

pub const EVENT_POWER_SHUTDOWN: u32 = 2;

// ── Event struct field indices ───────────────────────────────────────────────

/// Index: tag (u32) — PlatformEvent variant discriminator.
pub const EVENT_FIELD_TAG: usize = 0;

/// Index: app_class (string) — application ID (Focus, Block).
pub const EVENT_FIELD_APP_ID: usize = 1;

/// Index: title (string) — window title (Focus, Block).
pub const EVENT_FIELD_TITLE: usize = 2;

/// Index: power_tag (u32) — inner discriminator for PowerEvent (Suspend/Hibernate/Shutdown).
pub const EVENT_FIELD_POWER_TAG: usize = 3;

pub const EVENT_STRUCT_FIELD_COUNT: usize = 4;

// ═════════════════════════════════════════════════════════════════════════════
// D-Bus type signatures (cross-language contract)
//
// These string constants pin the D-Bus wire signatures that both Rust (zvariant)
// and C++ (sdbus-c++) must agree on.  Change with extreme care — the compositor
// plugin will get "Failed to enter a container" / "Failed to open a variant"
// serialization errors if these diverge.
// ═════════════════════════════════════════════════════════════════════════════

/// D-Bus struct signature for BlockedAppEntry: (string, int64, uint8, uint64).
/// `s` = AppClass (string), `x` = PolicyId (int64), `y` = BlockReason (uint8),
/// `t` = blocked_since (uint64).
/// Must match C++ tuple type in wellbeing_manager.cpp readBlockedApps.
pub const BLOCKED_APP_SIGNATURE: &str = "(sxyt)";

// ═════════════════════════════════════════════════════════════════════════════
// FocusChanged constants.
// ═════════════════════════════════════════════════════════════════════════════

pub const FOCUS_TAG_DESKTOP: u32 = 0;

pub const FOCUS_TAG_APP: u32 = 1;

pub const FOCUS_TAG_BLOCKED: u32 = 2;

pub const FOCUS_FIELD_TAG: usize = 0;

pub const FOCUS_FIELD_APP_ID: usize = 1;

pub const FOCUS_FIELD_TITLE: usize = 2;

pub const FOCUS_FIELD_PID: usize = 3;

pub const FOCUS_FIELD_UID: usize = 4;

pub const FOCUS_STRUCT_FIELD_COUNT: usize = 5;

pub const FOCUS_STRUCT_SIGNATURE: &str = "(ussuu)";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signal_names_match_controller_contract() {
        assert_eq!(APP_BLOCKED_SIGNAL, "AppBlocked");
        assert_eq!(POLICY_CHANGED_SIGNAL, "PolicyChanged");
        assert_eq!(DOMAIN_BLOCKED_SIGNAL, "DomainBlocked");
        assert_eq!(USAGE_UPDATED_SIGNAL, "UsageUpdated");
    }

    #[test]
    fn method_names_match_controller_contract() {
        assert_eq!(GET_BLOCKED_APPS_METHOD, "GetBlockedApps");
        assert_eq!(GET_BLOCKED_APPS_FOR_USER_METHOD, "GetBlockedAppsForUser");
        assert_eq!(REGISTER_BRIDGE_METHOD, "RegisterBridge");
    }

    #[test]
    fn blocked_app_signature_unchanged() {
        assert_eq!(BLOCKED_APP_SIGNATURE, "(sxyt)");
    }
}
