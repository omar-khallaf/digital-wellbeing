// =============================================================================
// WellbeingManager — D-Bus org.wellbeing.v1.Manager interface
//
// Implements the declarative architecture:
//   - Registers with daemon via RegisterPlugin
//   - Reads ActiveBlocks for initial overlay state
//   - Subscribes to BlockStateChanged for live updates
//   - Emits FocusChanged / ActivityChanged / UserAction signals
//   - Exposes CurrentFocus property
//   - Watches daemon bus name via NameOwnerChanged for auto-recovery
//
// The plugin connects to BOTH system and session D-Bus busses simultaneously
// (no probing, no background retry thread). The daemon bus is resolved at
// construction time and re-resolved when NameOwnerChanged fires.
//
// D-Bus calls to the daemon use C++20 coroutines (co_await) via
// sdbus-c++'s getResultAsAwaitable() API, driven by sdbus-c++'s
// internal event loop threads (enterEventLoopAsync).
//
// See docs/architecture/04-plugin-ipc.md and 05-daemon-auth.md.
// =============================================================================

#include "wellbeing_manager.hpp"

#include <cstdint>
#include <memory>
#include <optional>
#include <string>
#include <utility>
#include <vector>

#include <sdbus-c++/sdbus-c++.h>

#include "lockdown.hpp"
#include "logging.hpp"
#include "plugin_state.hpp"
#include "types.hpp"

using wellbeing::ActionType;
using wellbeing::AppId;
using wellbeing::BlockReason;
using wellbeing::FocusActivityTag;
using wellbeing::FocusVariantTag;
using wellbeing::g_ctx;
using wellbeing::logErr;
using wellbeing::logInfo;

namespace {

// =============================================================================
// WindowInfo helpers (D-Bus variant encoding)
// =============================================================================

/// Encode an Option<WindowInfo> as a D-Bus variant for FocusChanged.
///
/// Encoding:
///   None          → variant(uint32 FocusVariantTag::Desktop)
///   Some{...}     → variant(struct{FocusVariantTag::App, app_id, title, pid,
///                                   uid, overlay_shown})
auto windowInfoToVariant(const std::optional<WindowInfo> &info) -> sdbus::Variant {
    if (!info.has_value()) {
        return sdbus::Variant{static_cast<uint32_t>(FocusVariantTag::Desktop)};
    }
    // Use sdbus::Struct (not std::tuple) inside the Variant: sdbus::Struct
    // opens a D-Bus struct container, which the variant's signature `(ussuub)`
    // requires. std::tuple writes fields flat (no struct container), which
    // causes sd_bus_message_open_container to fail with EINVAL.
    return sdbus::Variant{sdbus::Struct{
        static_cast<uint32_t>(FocusVariantTag::App),
        info->appId.value(),
        info->title,
        info->pid,
        info->uid,
        info->overlayShown,
    }};
}

/// Build the CurrentFocus variant.
///
/// The CurrentFocus readable property MUST encode exactly what the
/// FocusChanged signal carries, so a late-joining client that missed the
/// ephemeral signal can read identical state from the property. Both call
/// windowInfoToVariant(currentFocus): no focus → Desktop (tag 0); focused app
/// → App (tag 1) with overlay_shown reflecting whether that app is blocked.
auto buildCurrentFocusVariant() -> sdbus::Variant { return windowInfoToVariant(g_ctx->currentFocus); }

} // anonymous namespace

// =============================================================================
// Daemon bus resolution helpers (4-step, using a held connection)
// =============================================================================

namespace {

/// Probe whether `name` is owned on `conn` via org.freedesktop.DBus.NameHasOwner.
auto nameHasOwner(sdbus::IConnection &conn, const std::string &name) -> bool {
    try {
        auto proxy = sdbus::createProxy(conn, sdbus::ServiceName{wellbeing::DBUS_INTERFACE},
                                        sdbus::ObjectPath{wellbeing::DBUS_OBJECT_PATH});
        bool owned = false;
        proxy->callMethod(wellbeing::NAME_HAS_OWNER_METHOD)
            .onInterface(wellbeing::DBUS_INTERFACE)
            .withArguments(name)
            .storeResultsTo(owned);
        return owned;
    } catch (const sdbus::Error &) {
        return false;
    }
}

/// Activate a service by calling org.freedesktop.DBus.StartServiceByName.
auto startServiceByName(sdbus::IConnection &conn, const std::string &name) -> bool {
    try {
        auto proxy = sdbus::createProxy(conn, sdbus::ServiceName{wellbeing::DBUS_INTERFACE},
                                        sdbus::ObjectPath{wellbeing::DBUS_OBJECT_PATH});
        uint32_t result = 0;
        proxy->callMethod(wellbeing::START_SERVICE_BY_NAME_METHOD)
            .onInterface(wellbeing::DBUS_INTERFACE)
            .withArguments(name, 0U)
            .storeResultsTo(result);
        return result == 1 || result == 2;
    } catch (const sdbus::Error &) {
        return false;
    }
}

} // anonymous namespace

// =============================================================================
// WellbeingManager
// =============================================================================

WellbeingManager::WellbeingManager(std::shared_ptr<LockManager> lockManager,
                                   std::shared_ptr<sdbus::IConnection> sysConnection,
                                   std::shared_ptr<sdbus::IConnection> sessConnection)
    : m_sysConn(std::move(sysConnection)), m_sessConn(std::move(sessConnection)),
      m_object(sdbus::createObject(*m_sysConn, sdbus::ObjectPath{wellbeing::MANAGER_OBJECT_PATH})),
      m_sessObject(sdbus::createObject(*m_sessConn, sdbus::ObjectPath{wellbeing::MANAGER_OBJECT_PATH})),
      m_lockManager(std::move(lockManager)), m_activeBus(resolveActiveDaemonBus(wellbeing::DAEMON_INTERFACE)),
      m_daemonBusName(daemonBusName()) {
    // ── VTable on system bus object ────────────────────────────────────
    m_object
        ->addVTable(sdbus::registerProperty(wellbeing::CURRENT_FOCUS_PROPERTY).withGetter([]() -> sdbus::Variant {
            return buildCurrentFocusVariant();
        }),
                    sdbus::registerSignal(wellbeing::USER_ACTION_SIGNAL)
                        .withParameters<std::string, uint32_t>({"app_id", "action"}),
                    sdbus::registerSignal(wellbeing::FOCUS_CHANGED_SIGNAL).withParameters<sdbus::Variant>({"window"}),
                    sdbus::registerSignal(wellbeing::ACTIVITY_CHANGED_SIGNAL).withParameters<uint32_t>({"activity"}))
        .forInterface(wellbeing::MANAGER_INTERFACE);

    // ── VTable on session bus object (same interface, same properties) ─
    m_sessObject
        ->addVTable(sdbus::registerProperty(wellbeing::CURRENT_FOCUS_PROPERTY).withGetter([]() -> sdbus::Variant {
            return buildCurrentFocusVariant();
        }),
                    sdbus::registerSignal(wellbeing::USER_ACTION_SIGNAL)
                        .withParameters<std::string, uint32_t>({"app_id", "action"}),
                    sdbus::registerSignal(wellbeing::FOCUS_CHANGED_SIGNAL).withParameters<sdbus::Variant>({"window"}),
                    sdbus::registerSignal(wellbeing::ACTIVITY_CHANGED_SIGNAL).withParameters<uint32_t>({"activity"}))
        .forInterface(wellbeing::MANAGER_INTERFACE);

    m_lockManager->setUserActionCallback(
        [this](const AppId &appId, ActionType action) -> void { emitUserAction(appId.value(), action); });

    // ── Resolve active daemon bus ──────────────────────────────────────

    // ── Create daemon proxy on the active bus (if found) ──────────────
    if (m_activeBus != DaemonBus::None) {
        auto &conn = (m_activeBus == DaemonBus::System) ? *m_sysConn : *m_sessConn;
        try {
            m_daemonProxy = sdbus::createProxy(conn, sdbus::ServiceName{m_daemonBusName},
                                               sdbus::ObjectPath{wellbeing::DAEMON_OBJECT_PATH});
        } catch (const sdbus::Error &e) {
            logErr("WellbeingManager: failed to create daemon proxy: " + std::string(e.what()));
        }
    }

    // ── Start event loops on BOTH connections ──────────────────────────
    try {
        m_sysConn->enterEventLoopAsync();
    } catch (const sdbus::Error &e) {
        logErr("WellbeingManager: failed to start system event loop: " + std::string(e.what()));
    }
    try {
        m_sessConn->enterEventLoopAsync();
    } catch (const sdbus::Error &e) {
        logErr("WellbeingManager: failed to start session event loop: " + std::string(e.what()));
    }

    // ── NameOwnerChanged watchers on BOTH connections ──────────────────
    setupNameOwnerWatch(true);  // system bus
    setupNameOwnerWatch(false); // session bus

    // ── Reverse-discovery + initial state sync ────────────────────────
    // Both registerWithDaemonAsync and readActiveBlocksAsync are coroutines
    // that start immediately (FireAndForget). readActiveBlocksAsync may fail if
    // the daemon is not yet available, which is fine — it retries on focus events.
    if (m_daemonProxy) {
        registerWithDaemonAsync();
        readActiveBlocksAsync();
    } else {
        logInfo("WellbeingManager: daemon not reachable on either bus — waiting for NameOwnerChanged");
    }
}

WellbeingManager::~WellbeingManager() {
    // Both watch slots (sdbus::Slot) are destroyed automatically as members,
    // which unsubscribes each NameOwnerChanged match.
    // Stop the internal event loop threads.
    m_sysConn->leaveEventLoop();
    m_sessConn->leaveEventLoop();
}

auto WellbeingManager::registerWithDaemonAsync() -> FireAndForget {
    if (!m_daemonProxy) {
        logErr("registerWithDaemonAsync: no daemon proxy available");
        co_return;
    }
    try {
        co_await m_daemonProxy->callMethodAsync(wellbeing::REGISTER_PLUGIN_METHOD)
            .onInterface(wellbeing::DAEMON_INTERFACE)
            .getResultAsAwaitable();
        logInfo("registerWithDaemonAsync: registered plugin instance");
    } catch (const sdbus::Error &e) {
        logInfo("registerWithDaemonAsync: daemon not reachable (" + std::string(e.what()) + ")");
    }
}

auto WellbeingManager::readActiveBlocksAsync() -> FireAndForget {
    if (!m_daemonProxy) {
        logErr("readActiveBlocksAsync: no daemon proxy");
        co_return;
    }

    try {
        // sdbus::Struct (NOT std::tuple) — signature_of adds () struct delimiters.
        // std::tuple's signature_of omits (), producing "astutau" (invalid D-Bus).
        using BlockEntry = sdbus::Struct<std::string, uint64_t, uint32_t, uint64_t, std::vector<uint32_t>>;
        using BlockEntries = std::vector<BlockEntry>;

        // Properties.Get returns v(a(stutau)) — Use getProperty + Variant::get<>()
        // (canonical sdbus-c++ property-read pattern).
        auto var = m_daemonProxy->getProperty("ActiveBlocks").onInterface(wellbeing::DAEMON_INTERFACE);
        auto blocks = var.get<BlockEntries>();

        for (auto &block : blocks) {
            auto &rawAppId = std::get<0>(block);
            auto policyId = std::get<1>(block);
            auto reason = std::get<2>(block);
            auto blockedSince = std::get<3>(block);
            auto &actions = std::get<4>(block);

            auto appId = AppId::from_raw(rawAppId);
            if (!appId.has_value()) {
                logErr("readActiveBlocksAsync: invalid appId '" + rawAppId + "' skipped");
                continue;
            }

            auto br = wellbeing::raw_to_block_reason(reason);
            if (!br.has_value()) {
                logErr("readActiveBlocksAsync: invalid BlockReason " + std::to_string(reason) + " skipped");
                continue;
            }

            std::vector<ActionType> typedActions;
            typedActions.reserve(actions.size());
            for (auto a : actions) {
                auto at = wellbeing::raw_to_action_type(a);
                if (at.has_value()) {
                    typedActions.push_back(*at);
                }
            }

            m_lockManager->showOverlay(*appId, policyId, *br, blockedSince, typedActions);

            if (g_ctx->currentFocus.has_value() && g_ctx->currentFocus->appId == *appId) {
                g_ctx->currentFocus->overlayShown = true;
            }
        }
    } catch (const sdbus::Error &e) {
        logInfo("readActiveBlocksAsync: daemon not available yet (" + std::string(e.what()) + ")");
    }
}

// ── Daemon bus resolution (4-step) ─────────────────────────────────

auto WellbeingManager::resolveActiveDaemonBus(const std::string &daemonBusName) -> DaemonBus {
    logInfo("resolveActiveDaemonBus: resolving daemon bus (4-step)");

    // Step 1: NameHasOwner on system bus
    if (m_sysConn && nameHasOwner(*m_sysConn, daemonBusName)) {
        logInfo("resolveActiveDaemonBus: daemon found on system bus (step 1)");
        return DaemonBus::System;
    }

    // Step 2: NameHasOwner on session bus
    if (m_sessConn && nameHasOwner(*m_sessConn, daemonBusName)) {
        logInfo("resolveActiveDaemonBus: daemon found on session bus (step 2)");
        return DaemonBus::Session;
    }

    // Step 3: StartServiceByName on system bus
    if (m_sysConn && startServiceByName(*m_sysConn, daemonBusName)) {
        logInfo("resolveActiveDaemonBus: daemon activated on system bus (step 3)");
        return DaemonBus::System;
    }

    // Step 4: StartServiceByName on session bus
    if (m_sessConn && startServiceByName(*m_sessConn, daemonBusName)) {
        logInfo("resolveActiveDaemonBus: daemon activated on session bus (step 4)");
        return DaemonBus::Session;
    }

    logInfo("resolveActiveDaemonBus: daemon not found on either bus");
    return DaemonBus::None;
}

// ── NameOwnerChanged match setup ────────────────────────────────────

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

// ── Daemon recovery via NameOwnerChanged ────────────────────────────

void WellbeingManager::onDaemonDisappeared() {
    m_activeBus = DaemonBus::None;
    m_daemonProxy.reset();
    logInfo("onDaemonDisappeared: daemon connection lost — waiting for reappearance");
}

void WellbeingManager::reconnectToDaemon() {
    auto resolved = resolveActiveDaemonBus(wellbeing::DAEMON_INTERFACE);
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

    // Re-register plugin instance — the daemon may have restarted.
    // Uses the async variant to avoid a D-Bus deadlock: the daemon calls
    // back to the plugin (CurrentFocus property) during registration, and a
    // synchronous callMethod would block the event loop thread, preventing
    // that callback from being dispatched.
    registerWithDaemonAsync();

    // Re-read all active blocks to synchronise overlay state (non-blocking coroutine).
    readActiveBlocksAsync();
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

// ── Signal emission ────────────────────────────────────────────────

/// Emit UserAction(app_id, action) on BOTH busses so the daemon receives it
/// regardless of which bus it is connected to.
void WellbeingManager::emitUserAction(const std::string &appId, ActionType action) {
    m_object->emitSignal(wellbeing::USER_ACTION_SIGNAL)
        .onInterface(wellbeing::MANAGER_INTERFACE)
        .withArguments(appId, static_cast<uint32_t>(action));
    m_sessObject->emitSignal(wellbeing::USER_ACTION_SIGNAL)
        .onInterface(wellbeing::MANAGER_INTERFACE)
        .withArguments(appId, static_cast<uint32_t>(action));
}

/// Emit FocusChanged(variant) on BOTH busses.
void WellbeingManager::emitFocusChanged(const std::optional<WindowInfo> &info) {
    m_object->emitSignal(wellbeing::FOCUS_CHANGED_SIGNAL)
        .onInterface(wellbeing::MANAGER_INTERFACE)
        .withArguments(windowInfoToVariant(info));
    m_sessObject->emitSignal(wellbeing::FOCUS_CHANGED_SIGNAL)
        .onInterface(wellbeing::MANAGER_INTERFACE)
        .withArguments(windowInfoToVariant(info));
}

/// Emit ActivityChanged(tag) on BOTH busses.
void WellbeingManager::emitActivityChanged(FocusActivityTag tag) {
    m_object->emitSignal(wellbeing::ACTIVITY_CHANGED_SIGNAL)
        .onInterface(wellbeing::MANAGER_INTERFACE)
        .withArguments(static_cast<uint32_t>(tag));
    m_sessObject->emitSignal(wellbeing::ACTIVITY_CHANGED_SIGNAL)
        .onInterface(wellbeing::MANAGER_INTERFACE)
        .withArguments(static_cast<uint32_t>(tag));
}

auto WellbeingManager::daemonBusName() -> std::string {
    return wellbeing::DAEMON_INTERFACE; // "org.wellbeing.v1.Controller"
}
