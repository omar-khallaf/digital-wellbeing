//! Shared D-Bus constants for the Digital Wellbeing system.
//! Single source of truth for all bus names, object paths, signal names,
//! and property names used across daemon, GUI, and plugin IPC.

// ── Daemon (Controller) interface ────────────────────────────────────────────

/// Well-known D-Bus interface name for the daemon's Controller API.
pub const DAEMON_INTERFACE: &str = "org.wellbeing.v1.Controller";

/// Object path where the Controller interface is registered.
pub const DAEMON_OBJECT_PATH: &str = "/org/wellbeing/Controller";

/// Well-known bus name for the daemon.
pub const DAEMON_BUS_NAME: &str = "org.wellbeing.v1.Controller";

// ── Plugin (Manager) interface — unchanged ───────────────────────────────────

/// Well-known D-Bus interface name for the compositor plugin's Manager API.
pub const MANAGER_INTERFACE: &str = "org.wellbeing.v1.Manager";

/// Object path where the Manager interface is registered.
pub const MANAGER_OBJECT_PATH: &str = "/org/wellbeing/Manager";

// ── Signal names on the Controller interface ─────────────────────────────────

/// Emitted when a block is shown or removed (a.k.a. BlockStateChanged).
pub const BLOCK_STATE_CHANGED_SIGNAL: &str = "BlockStateChanged";

/// Emitted when daily usage data is updated.
pub const DAILY_USAGE_CHANGED_SIGNAL: &str = "DailyUsageChanged";

/// Emitted when a policy is created, updated, or deleted.
pub const POLICY_MUTATED_SIGNAL: &str = "PolicyMutated";

// ── Signal names on the Manager interface ────────────────────────────────────

/// Emitted by the plugin when the focused window changes.
pub const FOCUS_CHANGED_SIGNAL: &str = "FocusChanged";

/// Emitted by the plugin when user activity/idle state changes.
pub const ACTIVITY_CHANGED_SIGNAL: &str = "ActivityChanged";

/// Emitted by the plugin when the user interacts with a block overlay.
pub const USER_ACTION_SIGNAL: &str = "UserAction";

// ── Property names ───────────────────────────────────────────────────────────

/// Read-only property on the Manager interface exposing current session state.
pub const CURRENT_SESSION_PROPERTY: &str = "CurrentSession";
