#pragma once

// =============================================================================
// PluginState — RAII owner of all plugin singletons.
//
// Owns the compositor-thread state (LockManager, IdleTracker), the
// cross-thread channels (ThreadChannels), and the D-Bus thread.
// Created in PLUGIN_INIT, destroyed in PLUGIN_EXIT.
// =============================================================================

#include <memory>

#include "dbus_thread.hpp"
#include "idle_tracker.hpp"
#include "lockdown.hpp"

namespace wellbeing {

/// Single global owner — created in PLUGIN_INIT, destroyed in PLUGIN_EXIT.
struct PluginState {
    std::unique_ptr<LockManager> lockManager;
    std::unique_ptr<ThreadChannels> channels;
    std::unique_ptr<DbusThread> dbusThread;
    std::unique_ptr<IdleTracker> idleTracker;
};

/// Global singleton owned via unique_ptr. Installed before hooks fire,
/// reset after they stop — no manual delete needed.
inline std::unique_ptr<PluginState> g_ps;

} // namespace wellbeing
