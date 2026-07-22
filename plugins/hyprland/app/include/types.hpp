#pragma once

#include <cstdint>
#include <optional>
#include <string>

namespace wellbeing {

// =============================================================================
// D-Bus constants
// =============================================================================

// Daemon (Controller) interface — the daemon's D-Bus API surface.
inline constexpr auto DAEMON_INTERFACE = "org.wellbeing.v1.Controller";
inline constexpr auto DAEMON_OBJECT_PATH = "/org/wellbeing/Controller";

// Manager interface — the plugin's D-Bus API surface.
inline constexpr auto MANAGER_INTERFACE = "org.wellbeing.v1.Manager";
inline constexpr auto MANAGER_OBJECT_PATH = "/org/wellbeing/Manager";

// Signal names (Manager → daemon/GUI)
inline constexpr auto FOCUS_CHANGED_SIGNAL = "FocusChanged";
inline constexpr auto ACTIVITY_CHANGED_SIGNAL = "ActivityChanged";
inline constexpr auto USER_ACTION_SIGNAL = "UserAction";

// Property name (Manager → daemon/GUI)
inline constexpr auto CURRENT_FOCUS_PROPERTY = "CurrentFocus";

// Daemon methods (Controller interface)
inline constexpr auto REGISTER_PLUGIN_METHOD = "RegisterPlugin";

// org.freedesktop.DBus.Properties
inline constexpr auto GET_PROPERTY_METHOD = "Get";
inline constexpr auto PROPERTIES_INTERFACE = "org.freedesktop.DBus.Properties";

// org.freedesktop.DBus (well-known)
inline constexpr auto DBUS_INTERFACE = "org.freedesktop.DBus";
inline constexpr auto DBUS_OBJECT_PATH = "/org/freedesktop/DBus";
inline constexpr auto NAME_HAS_OWNER_METHOD = "NameHasOwner";
inline constexpr auto START_SERVICE_BY_NAME_METHOD = "StartServiceByName";
inline constexpr auto NAME_OWNER_CHANGED_SIGNAL_NAME = "NameOwnerChanged";

// ── AppId ─────────────────────────────────────────────────────────────────────
// Validated non-empty identifier for an application (e.g. "firefox").
// Validated at the D-Bus boundary; LockManager never sees an unvalidated value.
class AppId {
  public:
    /// Empty id acts as the "no overlay" sentinel (default for LockManager /
    /// WindowInfo members).
    AppId() = default;

    /// Factory: validates non-empty and no embedded null bytes.
    /// Returns std::nullopt on invalid input (zero-trust boundary gate).
    static auto from_raw(const std::string &raw) -> std::optional<AppId> {
        if (raw.empty() || raw.find('\0') != std::string::npos) {
            return std::nullopt;
        }
        return AppId(raw);
    }

    /// For known-valid values only (test constants, internal recovery).
    static auto from_unchecked(std::string raw) -> AppId { return AppId(std::move(raw)); }

    [[nodiscard]] auto value() const -> const std::string & { return m_value; }
    [[nodiscard]] auto empty() const -> bool { return m_value.empty(); }

    auto operator==(const AppId &o) const -> bool { return m_value == o.m_value; }
    auto operator!=(const AppId &o) const -> bool { return m_value != o.m_value; }
    auto operator<(const AppId &o) const -> bool { return m_value < o.m_value; }

  private:
    explicit AppId(std::string raw) : m_value(std::move(raw)) {}
    std::string m_value;
};

// ── ActionType ────────────────────────────────────────────────────────────────
// Discriminated action identifiers echoed back in UserAction signals.
// D-Bus serialized as uint32_t — validated at boundary via from_raw().
enum class ActionType : uint8_t {
    Extra = 0,
    Close = 1,
};

/// Factory: validates a D-Bus-deserialized uint32_t into ActionType.
/// Returns std::nullopt for out-of-range values (zero-trust boundary gate).
[[nodiscard]] inline auto raw_to_action_type(uint32_t raw) -> std::optional<ActionType> {
    switch (static_cast<ActionType>(raw)) {
    case ActionType::Extra:
    case ActionType::Close:
        return static_cast<ActionType>(raw);
    }
    return std::nullopt;
}

// ── BlockReason ──────────────────────────────────────────────────────────────
// Why an app was blocked. Serialized over D-Bus as uint32_t, validated at
// the boundary. Used in Overlay(show) payload and BlockStateChanged signal.
enum class BlockReason : uint8_t {
    AppTimeLimit = 0,
    CategoryTimeLimit = 1,
    AppBlock = 2,
    CategoryBlock = 3,
};

/// Factory: validates a D-Bus-deserialized uint32_t into BlockReason.
/// Returns std::nullopt for out-of-range values (zero-trust boundary gate).
[[nodiscard]] inline auto raw_to_block_reason(uint32_t raw) -> std::optional<BlockReason> {
    switch (static_cast<BlockReason>(raw)) {
    case BlockReason::AppTimeLimit:
    case BlockReason::CategoryTimeLimit:
    case BlockReason::AppBlock:
    case BlockReason::CategoryBlock:
        return static_cast<BlockReason>(raw);
    }
    return std::nullopt;
}

// ── FocusVariantTag ──────────────────────────────────────────────────────────
// D-Bus variant discriminator for FocusChanged signal (org.wellbeing.v1.Manager).
/// Must match Rust handler in daemon/src/platform/linux/manager.rs (Value::U32(0)
/// for desktop, Value::U32(1) as first struct field for app).
/// Zero-based: no desktop tag collision, Rust side checks for U32(0).
///
/// Cross-reference: Rust FOCUS_TAG_DESKTOP / FOCUS_TAG_APP in
/// crates/core/src/dbus_constants.rs.
enum class FocusVariantTag : uint8_t {
    Desktop = 0,
    App = 1,
};

// ── FocusActivityTag ───────────────────────────────────────────────────────────
// Discriminator for ActivityChanged signal replacing the old bool encoding.
// Idle=0 means user activity has stopped; Resumed=1 means activity resumed.
///
/// Cross-reference: Rust ACTIVITY_TAG_IDLE / ACTIVITY_TAG_RESUMED in
/// crates/core/src/dbus_constants.rs.
enum class FocusActivityTag : uint8_t {
    Idle = 0,
    Resumed = 1,
};

// ── FocusChanged app-struct field indices ─────────────────────────────────────
// When the FocusChanged variant carries an app window (FocusVariantTag::App),
// the inner struct fields are accessed by these indices on the Rust side.
//
// Cross-reference: Rust FOCUS_FIELD_TAG … FOCUS_FIELD_OVERLAY in
// crates/core/src/dbus_constants.rs.
// =============================================================================

/// Index of the variant-tag field.
inline constexpr size_t FOCUS_FIELD_TAG = 0;
/// Index of the app_id field.
inline constexpr size_t FOCUS_FIELD_APP_ID = 1;
/// Index of the window-title field.
inline constexpr size_t FOCUS_FIELD_TITLE = 2;
/// Index of the PID field.
inline constexpr size_t FOCUS_FIELD_PID = 3;
/// Index of the UID field.
inline constexpr size_t FOCUS_FIELD_UID = 4;
/// Index of the overlay-shown field.
inline constexpr size_t FOCUS_FIELD_OVERLAY = 5;
/// Total number of fields in the FocusChanged app struct.
inline constexpr size_t FOCUS_STRUCT_FIELD_COUNT = 6;

// ── D-Bus type signatures (cross-language contract) ───────────────────────────
// These strings pin the D-Bus wire signatures that both Rust (zvariant) and C++
// (sdbus-c++) must agree on. Change with extreme care — mismatches cause
// "Failed to enter a container" or "Failed to open a variant" serialization
// errors.
//
// Cross-reference: Rust ACTIVE_BLOCK_SIGNATURE / FOCUS_STRUCT_SIGNATURE in
// crates/core/src/dbus_constants.rs.
// =============================================================================

/// D-Bus struct signature for ActiveBlockEntry: (string, uint64, uint32, uint64, array<uint32>).
inline constexpr auto ACTIVE_BLOCK_SIGNATURE = "(stutau)";

/// D-Bus struct signature for FocusChanged app variant: (uint32, string, string, uint32, uint32, bool).
inline constexpr auto FOCUS_STRUCT_SIGNATURE = "(ussuub)";

} // namespace wellbeing
