#pragma once

// =============================================================================
// Cross-thread message types for compositor ↔ D-Bus communication.
//
// Uses std::variant with compile-time exhaustive dispatch (std::visit).
// All structs are kept flat and minimal for fast SPSC queue transfer.
// =============================================================================

#include <optional>
#include <string>
#include <variant>
#include <vector>

#include "types.hpp"

namespace wellbeing {

// ── std::visit helper (C++17) ────────────────────────────────────────────────

template<class... Ts>
struct Overloaded : Ts... {
    using Ts::operator()...;
};
template<class... Ts>
Overloaded(Ts...) -> Overloaded<Ts...>;

// ── D-Bus thread → Compositor (chan B) ──────────────────────────────────────

/// Per-app block command (from BlockedAppsChanged signal).
struct BlockCmd {
    BlockCmd() = default;
    BlockCmd(std::string wclass_, BlockReason reason_) : wclass(std::move(wclass_)), reason(reason_) {}
    std::string wclass;
    BlockReason reason = BlockReason::AppTimeLimit;
};

/// Per-app unblock command.
struct UnblockCmd {
    std::string wclass;
};

/// Full atomic replacement of blocked-apps state.
/// Used on initial sync and daemon reconnect.
struct SyncAllCmd {
    struct Entry {
        Entry(std::string wclass_, BlockReason reason_) : wclass(std::move(wclass_)), reason(reason_) {}
        std::string wclass;
        BlockReason reason;
    };
    std::vector<Entry> entries;
};

/// Discriminated union of all compositor-targeted commands.
using CompositorCommand = std::variant<BlockCmd, UnblockCmd, SyncAllCmd>;

// ── Compositor → D-Bus thread (chan C) ──────────────────────────────────────

/// Focus state update for a non-blocked window (or unfocus).
/// wclass is nullopt when no window is focused.
/// Sent from WINDOW_ACTIVE hook, wTitle-change hook, and when the focused
/// window becomes unblocked. For blocked-window transitions see BlockedFocus.
struct FocusUpdate {
    std::optional<std::string> wclass;
    std::string wTitle;
};

/// The focused window is now blocked — from a focus change to a blocked
/// window, or the currently focused window became blocked.
/// wclass is always present (a blocked window must exist).
struct BlockedFocus {
    std::string wclass;
    std::string wTitle;
};

/// Idle state transition — emitted from IdleTracker callback.
struct IdleChanged {
    bool idle;
};

/// Signals D-Bus thread to exit its event loop (PLUGIN_EXIT).
struct ShutdownMsg {};

/// Discriminated union of all D-Bus-thread-targeted messages.
using DbusMessage = std::variant<FocusUpdate, BlockedFocus, IdleChanged, ShutdownMsg>;

} // namespace wellbeing
