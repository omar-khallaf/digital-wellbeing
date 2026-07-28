#pragma once

// =============================================================================
// LockManager — single-threaded blocked-apps state on the compositor thread.
//
// All public methods are called exclusively from the compositor thread
// (Hyprland event loop). No mutexes, no cross-thread sharing.
//
// State is modified via apply(const CompositorCommand&) which is called
// from the wl_event_loop_add_fd callback when the D-Bus thread pushes
// updates through the SPSC queue.
//
// LockManager is a PURE DATA STORE — it stores which apps are blocked and
// why. All window-dependent logic (hit-testing, rendering) lives in hooks.cpp.
// =============================================================================

#include <unordered_map>

#include "messages.hpp"
#include "types.hpp"

namespace wellbeing {

// ── LockManager ─────────────────────────────────────────────────────────────

class LockManager {
  public:
    LockManager() = default;

    /// Apply a command from the D-Bus thread. Exhaustive dispatch via std::visit.
    void apply(const CompositorCommand &cmd);

    /// Whether the given window class is currently blocked.
    [[nodiscard]] auto isBlocked(const std::string &wclass) const -> bool { return m_blocks.contains(wclass); }

    /// Get the block reason for a blocked app (for rendering).
    [[nodiscard]] auto blockReason(const std::string &wclass) const -> const BlockReason * {
        auto it = m_blocks.find(wclass);
        return it != m_blocks.end() ? &it->second : nullptr;
    }

    /// Returns all currently blocked window classes.
    [[nodiscard]] auto allBlocked() const -> const std::unordered_map<std::string, BlockReason> & { return m_blocks; }

  private:
    std::unordered_map<std::string, BlockReason> m_blocks;
};

} // namespace wellbeing
