#pragma once

#include <cstdint>
#include <optional>
#include <string>

namespace wellbeing {

inline constexpr auto DAEMON_INTERFACE = "org.wellbeing.v1.Controller";
inline constexpr auto DAEMON_OBJECT_PATH = "/org/wellbeing/Controller";

inline constexpr auto MANAGER_INTERFACE = "org.wellbeing.v1.Manager";
inline constexpr auto MANAGER_OBJECT_PATH = "/org/wellbeing/Manager";

/// Unified Event signal name (replaces FocusChanged + ActivityChanged + power_event).
inline constexpr auto EVENT_SIGNAL = "Event";

inline constexpr auto BLOCKED_APPS_CHANGED_SIGNAL = "BlockedAppsChanged";

inline constexpr auto REGISTER_PLUGIN_METHOD = "RegisterPlugin";

inline constexpr auto DBUS_INTERFACE = "org.freedesktop.DBus";
inline constexpr auto DBUS_OBJECT_PATH = "/org/freedesktop/DBus";
inline constexpr auto NAME_HAS_OWNER_METHOD = "NameHasOwner";
inline constexpr auto START_SERVICE_BY_NAME_METHOD = "StartServiceByName";

// Validated non-empty identifier for an application (e.g. "firefox").
// Validated at the D-Bus boundary; LockManager never sees an unvalidated value.
class AppClass {
  public:
    /// Empty id acts as the "no overlay" sentinel (default for LockManager /
    /// WindowInfo members).
    AppClass() = default;

    /// Factory: validates non-empty and no embedded null bytes.
    /// Returns std::nullopt on invalid input (zero-trust boundary gate).
    static auto from_raw(const std::string &raw) -> std::optional<AppClass> {
        if (raw.empty() || raw.find('\0') != std::string::npos) {
            return std::nullopt;
        }
        return AppClass(raw);
    }

    /// For known-valid values only (test constants, internal recovery).
    static auto from_unchecked(std::string raw) -> AppClass { return AppClass(std::move(raw)); }

    [[nodiscard]] auto value() const -> const std::string & { return m_value; }
    [[nodiscard]] auto empty() const -> bool { return m_value.empty(); }

    auto operator==(const AppClass &o) const -> bool { return m_value == o.m_value; }
    auto operator!=(const AppClass &o) const -> bool { return m_value != o.m_value; }
    auto operator<(const AppClass &o) const -> bool { return m_value < o.m_value; }

  private:
    explicit AppClass(std::string raw) : m_value(std::move(raw)) {}
    std::string m_value;
};

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

// Cross-reference: EVENT_TAG_FOCUS … EVENT_TAG_POWER in
// crates/core/src/dbus_constants.rs.

/// Event tag values for the unified Event signal struct.
/// Must match Rust EVENT_TAG_FOCUS (=0) … EVENT_TAG_LOCKED (=7).
enum class EventTag : uint8_t {
    Focus = 0,
    Unfocus = 1,
    Block = 2,
    Idle = 3,
    Resume = 4,
    LogOut = 5,
    Power = 6,
    Locked = 7,
};

/// Inner power-state discriminator for EventTag::Power.
/// Must match Rust EVENT_POWER_SUSPEND (=0) … EVENT_POWER_SHUTDOWN (=2).
enum class PowerTag : uint8_t {
    Suspend = 0,
    Hibernate = 1,
    Shutdown = 2,
};

// The Event signal and CurrentFocus property carry a D-Bus struct `(ussuu)`.
// Cross-reference: Rust EVENT_FIELD_TAG … EVENT_FIELD_POWER_TAG in
// crates/core/src/dbus_constants.rs.

inline constexpr size_t EVENT_FIELD_TAG = 0;
inline constexpr size_t EVENT_FIELD_APP_CLASS = 1;
inline constexpr size_t EVENT_FIELD_APP_TITLE = 2;
inline constexpr size_t EVENT_FIELD_PID = 3;
inline constexpr size_t EVENT_FIELD_POWER_TAG = 4;
inline constexpr size_t EVENT_STRUCT_FIELD_COUNT = 5;

// These strings pin the D-Bus wire signatures that both Rust (zvariant) and C++
// (sdbus-c++) must agree on. Change with extreme care — mismatches cause
// serialization errors.
// Cross-reference: Rust BLOCKED_APP_SIGNATURE / EVENT_STRUCT_SIGNATURE in
// crates/core/src/dbus_constants.rs.

/// (string, int64, uint32, uint64) — PolicyId is int64 (`x`) on the wire.
/// Cross-reference: Rust BLOCKED_APP_SIGNATURE in crates/core/src/dbus_constants.rs.
inline constexpr auto BLOCKED_APP_SIGNATURE = "(sxut)";

/// (uint32, string, string, uint32, uint32).
inline constexpr auto EVENT_STRUCT_SIGNATURE = "(ussuu)";

} // namespace wellbeing
