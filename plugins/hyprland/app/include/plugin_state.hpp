#pragma once

#include <memory>
#include <mutex>
#include <optional>

#include <hyprland/desktop/DesktopTypes.hpp>
#include <sdbus-c++/sdbus-c++.h>

#include "idle_tracker.hpp"
#include "lockdown.hpp"
#include "wellbeing_manager.hpp"

namespace wellbeing {

// PluginState — RAII owner of all plugin singletons

struct PluginState {
    std::shared_ptr<LockManager> lockManager;
    std::shared_ptr<sdbus::IConnection> sysConnection;
    std::shared_ptr<sdbus::IConnection> sessConnection;
    std::unique_ptr<WellbeingManager> wellbeingManager;

    // ── Canonical focus state (serialized over D-Bus as Event signal) ─
    // Mutex protects focusState against concurrent access from Hyprland
    // compositor thread (writes in WINDOW_FOCUS_HOOK / WINDOW_TITLE_HOOK)
    // and D-Bus event-loop thread (reads in CurrentFocus property getter).
    std::mutex m_focusMutex;
    std::optional<WindowInfo> focusState;

    // ── Cached uid for FocusChanged signals ──────────────────────────
    uint32_t uid = 0;

    // ── Internal Hyprland window handle (weak ref, compositor API queries) ─
    PHLWINDOWREF focusedHyprWindow;

    std::unique_ptr<IdleTracker> idleTracker;
};

/// Single global owner — created in PLUGIN_INIT, destroyed in PLUGIN_EXIT.
/// Never moved after creation; raw pointer access from hooks is safe.
inline std::unique_ptr<PluginState> g_ctx;

/// Thread-safe snapshot of focusState. Returns a copy under the mutex lock
/// so callers on any thread (Hyprland compositor or D-Bus) get a consistent
/// point-in-time view without holding the lock during downstream work
/// (variant serialization, signal emission).
inline auto focusStateSnapshot() -> std::optional<WindowInfo> {
    std::scoped_lock lock(g_ctx->m_focusMutex);
    return g_ctx->focusState;
}

} // namespace wellbeing
