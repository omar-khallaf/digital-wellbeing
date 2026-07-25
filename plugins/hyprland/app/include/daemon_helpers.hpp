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

/// Encode an Option<WindowInfo> as a D-Bus variant for the unified Event
/// signal / CurrentFocus property.
///
/// The payload is an `sdbus::Struct` with signature `(ussuu)`:
///
///   field | type   | contents
///   ------+--------+-----------------------------------------------
///   0     | u32    | event tag (EventTag enum value)
///   1     | string | app_id (Focus, Block)
///   2     | string | title  (Focus, Block)
///   3     | u32    | pid    (Focus)
///   4     | u32    | power_tag  (PowerEvent: Suspend / Hibernate / Shutdown)
///
/// Encoding:
///   None (no focus)     → {EventTag::Unfocus, "", "", 0, 0}
///   Some{unblocked}     → {EventTag::Focus, appId, title, pid, 0}
///   Some{blocked}       → {EventTag::Block, appId, title, pid, 0}
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
