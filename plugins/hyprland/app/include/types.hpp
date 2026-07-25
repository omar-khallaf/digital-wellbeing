#pragma once

#include <cstdint>
#include <optional>
#include <string>

namespace wellbeing {

// ── D-Bus constants ──────────────────────────────────────────────────────────────

inline constexpr auto DAEMON_INTERFACE = "org.wellbeing.v1.Controller";
inline constexpr auto DAEMON_OBJECT_PATH = "/org/wellbeing/Controller";

inline constexpr auto MANAGER_INTERFACE = "org.wellbeing.v1.Manager";
inline constexpr auto MANAGER_OBJECT_PATH = "/org/wellbeing/Manager";

inline constexpr auto FOCUS_CHANGED_SIGNAL = "FocusChanged";
inline constexpr auto ACTIVITY_CHANGED_SIGNAL = "ActivityChanged";
// UserAction signal removed — close button handled locally in plugin.
// See docs/architecture/04-plugin-ipc.md for the handshake protocol.

inline constexpr auto BLOCKED_APPS_CHANGED_SIGNAL = "BlockedAppsChanged";

inline constexpr auto REGISTER_PLUGIN_METHOD = "RegisterPlugin";

inline constexpr auto GET_PROPERTY_METHOD = "Get";
inline constexpr auto PROPERTIES_INTERFACE = "org.freedesktop.DBus.Properties";

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
// Serialized over D-Bus as uint32_t — validated at boundary via raw_to_action_type().
enum class ActionType : uint8_t {
    Close = 1,
};

[[nodiscard]] inline auto raw_to_action_type(uint32_t raw) -> std::optional<ActionType> {
    switch (static_cast<ActionType>(raw)) {
    case ActionType::Close:
        return static_cast<ActionType>(raw);
    }
    return std::nullopt;
}

// ── BlockReason ──────────────────────────────────────────────────────────────
// Serialized over D-Bus as uint32_t — validated at the boundary.
enum class BlockReason : uint8_t {
    AppTimeLimit = 0,
    CategoryTimeLimit = 1,
    AppBlock = 2,
    CategoryBlock = 3,
};

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
// D-Bus variant discriminator for FocusChanged signal.
/// Cross-reference: Rust FOCUS_TAG_DESKTOP / FOCUS_TAG_APP in
/// crates/core/src/dbus_constants.rs.
enum class FocusVariantTag : uint8_t {
    Desktop = 0,
    App = 1,
    Blocked = 2,
};

// ── FocusActivityTag ───────────────────────────────────────────────────────────
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
// the inner struct fields are accessed by these field indices on the Rust side.
// Cross-reference: Rust FOCUS_FIELD_TAG … FOCUS_FIELD_OVERLAY in
// crates/core/src/dbus_constants.rs.

inline constexpr size_t FOCUS_FIELD_TAG = 0;
inline constexpr size_t FOCUS_FIELD_APP_ID = 1;
inline constexpr size_t FOCUS_FIELD_TITLE = 2;
inline constexpr size_t FOCUS_FIELD_PID = 3;
inline constexpr size_t FOCUS_FIELD_UID = 4;
inline constexpr size_t FOCUS_STRUCT_FIELD_COUNT = 5;

// ── D-Bus type signatures (cross-language contract) ───────────────────────────
// These strings pin the D-Bus wire signatures that both Rust (zvariant) and C++
// (sdbus-c++) must agree on. Change with extreme care — mismatches cause
// serialization errors.
// Cross-reference: Rust BLOCKED_APP_SIGNATURE / FOCUS_STRUCT_SIGNATURE in
// crates/core/src/dbus_constants.rs.

/// (string, uint64, uint32, uint64) — no actions vector, close button handled locally.
inline constexpr auto BLOCKED_APP_SIGNATURE = "(stut)";

/// (uint32, string, string, uint32, uint32).
inline constexpr auto FOCUS_STRUCT_SIGNATURE = "(ussuu)";

} // namespace wellbeing
