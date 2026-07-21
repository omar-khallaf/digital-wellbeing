#pragma once

#include <atomic>
#include <chrono>
#include <memory>
#include <optional>

#include <sdbus-c++/sdbus-c++.h>

#include "lockdown.hpp"
#include "wellbeing_manager.hpp"

namespace wellbeing {

// =============================================================================
// PluginState — RAII owner of all plugin singletons
// =============================================================================

struct PluginState {
    std::shared_ptr<LockManager> lockManager;
    std::shared_ptr<sdbus::IConnection> dbusConnection;
    std::unique_ptr<WellbeingManager> manager;
    std::optional<WindowInfo> currentFocus;

    // ── Cached uid for FocusChanged signals ──────────────────────────
    uint32_t uid = 0;

    // ── Activity tracking state ──────────────────────────────────────
    bool idle = false;
    std::chrono::steady_clock::time_point lastActivity;
};

/// Single global owner — created in PLUGIN_INIT, destroyed in PLUGIN_EXIT.
/// Never moved after creation; raw pointer access from hooks is safe.
inline std::unique_ptr<PluginState> g_ctx;

} // namespace wellbeing
