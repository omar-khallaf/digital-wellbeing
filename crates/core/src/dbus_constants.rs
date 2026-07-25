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

// ── Plugin (Manager) interface ───────────────────────────────────────────────

/// Well-known D-Bus interface name for the compositor plugin's Manager API.
pub const MANAGER_INTERFACE: &str = "org.wellbeing.v1.Manager";

/// Object path where the Manager interface is registered.
pub const MANAGER_OBJECT_PATH: &str = "/org/wellbeing/Manager";

// ── Signal names on the Controller interface ─────────────────────────────────

/// Emitted when a block is shown or removed (a.k.a. BlockedAppsChanged).
pub const BLOCKED_APPS_CHANGED_SIGNAL: &str = "BlockedAppsChanged";

/// Emitted when daily usage data is updated.
pub const DAILY_USAGE_CHANGED_SIGNAL: &str = "DailyUsageChanged";

/// Emitted when a policy is created, updated, or deleted.
pub const POLICY_MUTATED_SIGNAL: &str = "PolicyMutated";

// ── Signal names on the Manager interface ────────────────────────────────────

/// Name of the unified `event` signal (replaces FocusChanged + ActivityChanged).
pub const EVENT_SIGNAL: &str = "Event";

// ── Property names ───────────────────────────────────────────────────────────

/// Read-only property on the Manager interface exposing current session state.
pub const CURRENT_SESSION_PROPERTY: &str = "CurrentSession";

// ═════════════════════════════════════════════════════════════════════════════
// Unified event signal — replaces FocusChanged, ActivityChanged, and power_event.
//
// The `event` signal carries a D-Bus struct with 5 fields:
//   (u:tag, s:app_id, s:title, u:pid, u:power_tag)
//
// Signature: `(ussuu)`

/// D-Bus struct signature for the unified event payload.
pub const EVENT_STRUCT_SIGNATURE: &str = "(ussuu)";

/// Event tag for Focus — a window received focus. Applies to `uid`.
/// Relevant fields: app_id, title, pid
pub const EVENT_TAG_FOCUS: u32 = 0;

/// Event tag for Unfocus — all windows for `uid` lost focus (desktop shown).
/// Relevant fields: uid only.
pub const EVENT_TAG_UNFOCUS: u32 = 1;

/// Event tag for Block — focus changed to a blocked window (overlay shown).
/// Relevant fields: app_id, title, uid.
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

/// Power-event inner tag for Suspend.
pub const EVENT_POWER_SUSPEND: u32 = 0;

/// Power-event inner tag for Hibernate.
pub const EVENT_POWER_HIBERNATE: u32 = 1;

/// Power-event inner tag for Shutdown.
pub const EVENT_POWER_SHUTDOWN: u32 = 2;

// ── Event struct field indices ───────────────────────────────────────────────

/// Index: tag (u32) — PlatformEvent variant discriminator.
pub const EVENT_FIELD_TAG: usize = 0;

/// Index: app_id (string) — application ID (Focus, Block).
pub const EVENT_FIELD_APP_ID: usize = 1;

/// Index: title (string) — window title (Focus, Block).
pub const EVENT_FIELD_TITLE: usize = 2;

/// Index: pid (u32) — process ID (Focus).
pub const EVENT_FIELD_PID: usize = 3;

/// Index: power_tag (u32) — inner discriminator for PowerEvent (Suspend/Hibernate/Shutdown).
pub const EVENT_FIELD_POWER_TAG: usize = 4;

/// Total number of fields in the event struct.
pub const EVENT_STRUCT_FIELD_COUNT: usize = 5;

// ═════════════════════════════════════════════════════════════════════════════
// D-Bus type signatures (cross-language contract)
//
// These string constants pin the D-Bus wire signatures that both Rust (zvariant)
// and C++ (sdbus-c++) must agree on.  Change with extreme care — the compositor
// plugin will get "Failed to enter a container" / "Failed to open a variant"
// serialization errors if these diverge.
// ═════════════════════════════════════════════════════════════════════════════

/// D-Bus struct signature for BlockedAppEntry: (string, uint64, uint32, uint64).
/// Must match C++ tuple type in wellbeing_manager.cpp readBlockedApps.
pub const BLOCKED_APP_SIGNATURE: &str = "(stut)";

// ═════════════════════════════════════════════════════════════════════════════
// Legacy FocusChanged constants — retained during migration, no longer emitted by the plugin.
// ═════════════════════════════════════════════════════════════════════════════

/// Legacy: FocusChanged variant U32 value — desktop/unfocused.
pub const FOCUS_TAG_DESKTOP: u32 = 0;

/// Legacy: FocusChanged struct first-field — app variant discriminator.
pub const FOCUS_TAG_APP: u32 = 1;

/// Legacy: FocusChanged variant U32 value — window blocked by enforcement.
pub const FOCUS_TAG_BLOCKED: u32 = 2;

/// Legacy: Index of the variant-tag field in the FocusChanged struct.
pub const FOCUS_FIELD_TAG: usize = 0;

/// Legacy: Index of the app_id field in the FocusChanged struct.
pub const FOCUS_FIELD_APP_ID: usize = 1;

/// Legacy: Index of the window-title field in the FocusChanged struct.
pub const FOCUS_FIELD_TITLE: usize = 2;

/// Legacy: Index of the PID field in the FocusChanged struct.
pub const FOCUS_FIELD_PID: usize = 3;

/// Legacy: Index of the UID field in the FocusChanged struct.
pub const FOCUS_FIELD_UID: usize = 4;

/// Legacy: Total number of fields in the FocusChanged app struct.
pub const FOCUS_STRUCT_FIELD_COUNT: usize = 5;

/// Legacy: D-Bus struct signature for FocusChanged app variant: (u, s, s, u, u).
pub const FOCUS_STRUCT_SIGNATURE: &str = "(ussuu)";
