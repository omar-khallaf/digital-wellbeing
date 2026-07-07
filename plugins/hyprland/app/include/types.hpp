#pragma once

#include <cstdint>
#include <optional>
#include <string>

namespace wellbeing {

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

// ── FocusVariantTag ───────────────────────────────────────────────────────────
// D-Bus variant discriminator for FocusChanged signal (org.wellbeing.v1.Manager).
enum class FocusVariantTag : uint8_t {
    Desktop = 1,
    App = 2,
};

// CurrentSession reuses FocusVariantTag (see above) — no separate SessionStateTag.

} // namespace wellbeing
