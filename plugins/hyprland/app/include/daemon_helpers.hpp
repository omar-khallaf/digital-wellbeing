#pragma once

// =============================================================================
// Daemon bus resolution helpers — free functions shared across the plugin.
//
// These were extracted from wellbeing_manager.cpp to reduce its size (491 LOC)
// and to clarify the boundary between shared D-Bus utility code and
// WellbeingManager's stateful orchestration.
//
// See docs/architecture/04-plugin-ipc.md and 05-daemon-auth.md.
// =============================================================================

#include <optional>

#include <sdbus-c++/sdbus-c++.h>

#include "lockdown.hpp"

namespace wellbeing {

/// Encode an Option<WindowInfo> as a D-Bus variant for FocusChanged.
///
/// Encoding:
///   None                → variant(uint32 FocusVariantTag::Desktop)
///   Some{unblocked}     → variant(struct{FocusVariantTag::App, app_id, title, pid, uid})
///   Some{blocked}       → variant(struct{FocusVariantTag::Blocked, app_id, title, pid, uid})
auto windowInfoToVariant(const std::optional<WindowInfo> &info, bool blocked) -> sdbus::Variant;

/// Check whether a D-Bus bus name has an owner.
/// Returns false on any error (name not found, bus unreachable, etc.).
auto nameHasOwner(sdbus::IConnection &conn, const std::string &name) -> bool;

/// Activate a D-Bus service by name (equivalent to dbus-send --print-reply
/// --dest=org.freedesktop.DBus /org/freedesktop/DBus
/// org.freedesktop.DBus.StartServiceByName string:<name> uint32:0).
/// Returns true if the service was started successfully (already running or
/// just activated).
auto startServiceByName(sdbus::IConnection &conn, const std::string &name) -> bool;

} // namespace wellbeing
