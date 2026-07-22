#pragma once

#include <memory>
#include <optional>

#include <hyprland/desktop/DesktopTypes.hpp>
#include <sdbus-c++/sdbus-c++.h>

#include "idle_tracker.hpp"
#include "lockdown.hpp"
#include "wellbeing_manager.hpp"

namespace wellbeing {

// =============================================================================
// PluginState — RAII owner of all plugin singletons
// =============================================================================

struct PluginState {
    std::shared_ptr<LockManager> lockManager;
    std::shared_ptr<sdbus::IConnection> sysConnection;
    std::shared_ptr<sdbus::IConnection> sessConnection;
    std::unique_ptr<WellbeingManager> manager;
    std::optional<WindowInfo> currentFocus;

    // ── Cached uid for FocusChanged signals ──────────────────────────
    uint32_t uid = 0;

    // ── Focused Hyprland window (weak ref, avoids iteration in inhibit check) ─
    PHLWINDOWREF focusedWindow;

    // ── Activity tracking state ──────────────────────────────────────
    std::unique_ptr<IdleTracker> idleTracker;
};

/// Single global owner — created in PLUGIN_INIT, destroyed in PLUGIN_EXIT.
/// Never moved after creation; raw pointer access from hooks is safe.
inline std::unique_ptr<PluginState> g_ctx;

} // namespace wellbeing
