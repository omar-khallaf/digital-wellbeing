// =============================================================================
// Daemon resolution — bus resolution + NameOwnerChanged recovery
// =============================================================================

#include "daemon_resolution.hpp"

#include "daemon_helpers.hpp"
#include "logging.hpp"

using wellbeing::logErr;
using wellbeing::logInfo;
using wellbeing::nameHasOwner;
using wellbeing::resolveActiveDaemonBus;
using wellbeing::startServiceByName;

auto wellbeing::resolveActiveDaemonBus(const std::string &daemonBusName,
                                       const std::shared_ptr<sdbus::IConnection> &sysConn,
                                       const std::shared_ptr<sdbus::IConnection> &sessConn)
    -> WellbeingManager::DaemonBus {
    logInfo("resolveActiveDaemonBus: resolving daemon bus (4-step)");

    if (sysConn && nameHasOwner(*sysConn, daemonBusName)) {
        logInfo("resolveActiveDaemonBus: daemon found on system bus (step 1)");
        return WellbeingManager::DaemonBus::System;
    }

    if (sessConn && nameHasOwner(*sessConn, daemonBusName)) {
        logInfo("resolveActiveDaemonBus: daemon found on session bus (step 2)");
        return WellbeingManager::DaemonBus::Session;
    }

    if (sysConn && startServiceByName(*sysConn, daemonBusName)) {
        logInfo("resolveActiveDaemonBus: daemon activated on system bus (step 3)");
        return WellbeingManager::DaemonBus::System;
    }

    if (sessConn && startServiceByName(*sessConn, daemonBusName)) {
        logInfo("resolveActiveDaemonBus: daemon activated on session bus (step 4)");
        return WellbeingManager::DaemonBus::Session;
    }

    logInfo("resolveActiveDaemonBus: daemon not found on either bus");
    return WellbeingManager::DaemonBus::None;
}

namespace wellbeing {

void WellbeingManager::setupNameOwnerWatch(bool system) {
    auto &conn = system ? *m_sysConn : *m_sessConn;
    auto &slot = system ? m_sysDaemonWatchSlot : m_sessDaemonWatchSlot;

    const auto matchExpr = std::string("type='signal',"
                                       "sender='org.freedesktop.DBus',"
                                       "interface='org.freedesktop.DBus',"
                                       "member='NameOwnerChanged',"
                                       "path='/org/freedesktop/DBus'");

    try {
        slot = conn.addMatch(
            matchExpr,
            [this, isSystem = system](sdbus::Message msg) -> void {
                std::string name;
                std::string oldOwner;
                std::string newOwner;
                msg >> name >> oldOwner >> newOwner;
                onNameOwnerChanged(name, oldOwner, newOwner, isSystem);
            },
            sdbus::return_slot);

        logInfo("setupNameOwnerWatch: watching NameOwnerChanged on " + std::string(system ? "system" : "session") +
                " bus for " + m_daemonBusName);
    } catch (const sdbus::Error &e) {
        logErr("setupNameOwnerWatch: addMatch failed on " + std::string(system ? "system" : "session") +
               " bus: " + std::string(e.what()));
    }
}

void WellbeingManager::onDaemonDisappeared() {
    m_activeBus = DaemonBus::None;
    m_daemonProxy.reset();
    logInfo("onDaemonDisappeared: daemon connection lost — waiting for reappearance");
}

void WellbeingManager::reconnectToDaemon() {
    auto resolved = resolveActiveDaemonBus(wellbeing::DAEMON_INTERFACE, m_sysConn, m_sessConn);
    if (resolved == DaemonBus::None) {
        logInfo("reconnectToDaemon: daemon still unreachable");
        return;
    }

    logInfo("reconnectToDaemon: daemon found on " + std::string(resolved == DaemonBus::System ? "system" : "session") +
            " bus — reconnecting");

    m_activeBus = resolved;

    // Re-create daemon proxy on the resolved connection.
    auto &conn = (m_activeBus == DaemonBus::System) ? *m_sysConn : *m_sessConn;
    try {
        m_daemonProxy = sdbus::createProxy(conn, sdbus::ServiceName{m_daemonBusName},
                                           sdbus::ObjectPath{wellbeing::DAEMON_OBJECT_PATH});
    } catch (const sdbus::Error &e) {
        logErr("reconnectToDaemon: failed to create daemon proxy: " + std::string(e.what()));
        m_activeBus = DaemonBus::None;
        return;
    }

    // Re-register, re-sync blocked apps, and emit current focus.
    // Uses the async variant to avoid a D-Bus deadlock: the daemon calls
    // back to the plugin (CurrentFocus property) during registration, and a
    // synchronous callMethod would block the event loop thread, preventing
    // that callback from being dispatched.
    handshake();

    setupBlockedAppsWatch();
}

void WellbeingManager::onDaemonAppeared() {
    logInfo("onDaemonAppeared: daemon bus name appeared — re-registering and syncing state");
    reconnectToDaemon();
}

void WellbeingManager::onNameOwnerChanged(const std::string &name, const std::string &oldOwner,
                                          const std::string &newOwner, bool isSystem) {
    if (name != m_daemonBusName) {
        return; // not our daemon
    }

    const auto *const busLabel = isSystem ? "system" : "session";

    if (!oldOwner.empty() && newOwner.empty()) {
        logInfo("onNameOwnerChanged: daemon '" + name + "' disappeared from " + busLabel + " bus");
        DaemonBus disappearedBus = isSystem ? DaemonBus::System : DaemonBus::Session;
        if (disappearedBus == m_activeBus) {
            onDaemonDisappeared();
            reconnectToDaemon();
        }
    } else if (oldOwner.empty() && !newOwner.empty()) {
        logInfo("onNameOwnerChanged: daemon '" + name + "' appeared on " + busLabel + " bus");
        reconnectToDaemon();
    } else if (!oldOwner.empty() && !newOwner.empty()) {
        logInfo("onNameOwnerChanged: daemon '" + name + "' changed owner on " + busLabel + " bus: " + oldOwner + " → " +
                newOwner);
        reconnectToDaemon();
    }
}

} // namespace wellbeing
