// =============================================================================
// Daemon resolution — bus resolution + NameOwnerChanged recovery
// =============================================================================

#pragma once

#include <memory>
#include <string>

#include <sdbus-c++/sdbus-c++.h>

#include "wellbeing_manager.hpp"

namespace wellbeing {

// Resolve the daemon bus using a 4-step strategy:
//   1. NameHasOwner on system bus
//   2. NameHasOwner on session bus
//   3. StartServiceByName on system bus
//   4. StartServiceByName on session bus
auto resolveActiveDaemonBus(const std::string &daemonBusName,
                            std::shared_ptr<sdbus::IConnection> sysConn,
                            std::shared_ptr<sdbus::IConnection> sessConn) -> WellbeingManager::DaemonBus;

} // namespace wellbeing
