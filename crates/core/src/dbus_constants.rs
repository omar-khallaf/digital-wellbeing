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

// ═════════════════════════════════════════════════════════════════════════════
// FocusChanged variant tags (Manager signal)
//
// The FocusChanged signal carries a D-Bus variant whose type discriminator
// determines whether the focused window is a desktop (no app) or an app.
//
// Must match C++ FocusVariantTag in plugins/hyprland/app/include/types.hpp.
// ═════════════════════════════════════════════════════════════════════════════

/// FocusChanged variant U32 value — desktop/unfocused (no app window).
pub const FOCUS_TAG_DESKTOP: u32 = 0;

/// FocusChanged variant struct first-field — app variant discriminator.
pub const FOCUS_TAG_APP: u32 = 1;

// ═════════════════════════════════════════════════════════════════════════════
// ActivityChanged tags (Manager signal)
//
// The ActivityChanged signal carries a plain u32 indicating idle or resumed.
// Must match C++ FocusActivityTag in plugins/hyprland/app/include/types.hpp.
// ═════════════════════════════════════════════════════════════════════════════

/// ActivityChanged u32 value — user activity stopped.
pub const ACTIVITY_TAG_IDLE: u32 = 0;

/// ActivityChanged u32 value — user activity resumed.
pub const ACTIVITY_TAG_RESUMED: u32 = 1;

// ═════════════════════════════════════════════════════════════════════════════
// FocusChanged app-variant struct field indices
//
// When FocusChanged carries an app window, the variant contains a D-Bus struct
// with these fields in order.  Used by the Rust handler in
// daemon/src/platform/linux/manager.rs to destructure the signal payload.
// ═════════════════════════════════════════════════════════════════════════════

/// Index of the variant-tag field (Value::U32(FOCUS_TAG_APP)).
pub const FOCUS_FIELD_TAG: usize = 0;

/// Index of the app_id field (Value::Str).
pub const FOCUS_FIELD_APP_ID: usize = 1;

/// Index of the window-title field (Value::Str).
pub const FOCUS_FIELD_TITLE: usize = 2;

/// Index of the PID field (Value::U32).
pub const FOCUS_FIELD_PID: usize = 3;

/// Index of the UID field (Value::U32).
pub const FOCUS_FIELD_UID: usize = 4;

/// Index of the overlay-shown field (Value::Bool).
pub const FOCUS_FIELD_OVERLAY: usize = 5;

/// Total number of fields in the FocusChanged app struct.
pub const FOCUS_STRUCT_FIELD_COUNT: usize = 6;

// ═════════════════════════════════════════════════════════════════════════════
// D-Bus type signatures (cross-language contract)
//
// These string constants pin the D-Bus wire signatures that both Rust (zvariant)
// and C++ (sdbus-c++) must agree on.  Change with extreme care — the compositor
// plugin will get "Failed to enter a container" / "Failed to open a variant"
// serialization errors if these diverge.
// ═════════════════════════════════════════════════════════════════════════════

/// D-Bus struct signature for ActiveBlockEntry: (string, uint64, uint32, uint64, array<uint32>).
/// Must match C++ tuple type in wellbeing_manager.cpp readActiveBlocks.
pub const ACTIVE_BLOCK_SIGNATURE: &str = "(stutau)";

/// D-Bus struct signature for FocusChanged app variant: (uint32, string, string, uint32, uint32, bool).
/// Must match C++ sdbus::Struct type in wellbeing_manager.cpp windowInfoToVariant.
pub const FOCUS_STRUCT_SIGNATURE: &str = "(ussuub)";
