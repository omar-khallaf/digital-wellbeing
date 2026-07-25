// =============================================================================
// WellbeingManager — D-Bus org.wellbeing.v1.Manager interface
//
// Implements the declarative architecture:
//   - Registers with daemon via RegisterPlugin
//   - Reads BlockedApps for initial overlay state
//   - Subscribes to BlockedAppsChanged for live updates
//   - Emits FocusChanged / ActivityChanged signals
//   - Close button handled locally (no UserAction signal)
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
///   None                → variant(uint32 FocusVariantTag::Desktop)
///   Some{unblocked}     → variant(struct{FocusVariantTag::App, app_id, title, pid, uid})
///   Some{blocked}       → variant(struct{FocusVariantTag::Blocked, app_id, title, pid, uid})
auto windowInfoToVariant(const std::optional<WindowInfo> &info, bool blocked) -> sdbus::Variant {
    if (!info.has_value()) {
        return sdbus::Variant{static_cast<uint32_t>(FocusVariantTag::Desktop)};
    }
    auto tag = blocked ? FocusVariantTag::Blocked : FocusVariantTag::App;
    return sdbus::Variant{sdbus::Struct{
        static_cast<uint32_t>(tag),
        info->appId.value(),
        info->title,
        info->pid,
        info->uid,
    }};
}

} // anonymous namespace

// =============================================================================
// Daemon bus resolution helpers (4-step, using a held connection)
// =============================================================================

namespace {

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
      m_sysObject(sdbus::createObject(*m_sysConn, sdbus::ObjectPath{wellbeing::MANAGER_OBJECT_PATH})),
      m_sessObject(sdbus::createObject(*m_sessConn, sdbus::ObjectPath{wellbeing::MANAGER_OBJECT_PATH})),
      m_lockManager(std::move(lockManager)), m_activeBus(resolveActiveDaemonBus(wellbeing::DAEMON_INTERFACE)),
      m_daemonBusName(daemonBusName()) {
    // ── VTable on system bus object ────────────────────────────────────
    m_sysObject
        ->addVTable(sdbus::registerSignal(wellbeing::FOCUS_CHANGED_SIGNAL).withParameters<sdbus::Variant>({"window"}),
                    sdbus::registerSignal(wellbeing::ACTIVITY_CHANGED_SIGNAL).withParameters<uint32_t>({"activity"}),
                    sdbus::registerProperty("CurrentFocus").withGetter([this]() -> sdbus::Variant {
                        bool blocked =
                            g_ctx->focusState.has_value() && m_lockManager->isOverlayShown(g_ctx->focusState->appId);
                        return windowInfoToVariant(g_ctx->focusState, blocked);
                    }))
        .forInterface(wellbeing::MANAGER_INTERFACE);

    // ── VTable on session bus object ──
    m_sessObject
        ->addVTable(sdbus::registerSignal(wellbeing::FOCUS_CHANGED_SIGNAL).withParameters<sdbus::Variant>({"window"}),
                    sdbus::registerSignal(wellbeing::ACTIVITY_CHANGED_SIGNAL).withParameters<uint32_t>({"activity"}),
                    sdbus::registerProperty("CurrentFocus").withGetter([this]() -> sdbus::Variant {
                        bool blocked =
                            g_ctx->focusState.has_value() && m_lockManager->isOverlayShown(g_ctx->focusState->appId);
                        return windowInfoToVariant(g_ctx->focusState, blocked);
                    }))
        .forInterface(wellbeing::MANAGER_INTERFACE);

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

    // ── Initial state sync ────────────────────────────────────────────
    // initialSync registers with the daemon, reads blocked apps, and emits
    // the current focus state — all asynchronously via coroutines.
    if (m_daemonProxy) {
        handshake();
        setupBlockedAppsWatch();
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

// ── Handshake: register → fetch blocks → emit ───────────────────────

auto WellbeingManager::handshake() -> fire_and_forget {
    if (!m_daemonProxy) {
        logErr("handshake: no daemon proxy");
        co_return;
    }

    try {
        co_await m_daemonProxy->callMethodAsync(wellbeing::REGISTER_PLUGIN_METHOD)
            .onInterface(wellbeing::DAEMON_INTERFACE)
            .getResultAsAwaitable();
        logInfo("handshake: registered plugin instance");
        m_registered = true;
    } catch (const sdbus::Error &e) {
        logInfo("handshake: daemon not reachable (" + std::string(e.what()) + ")");
        co_return;
    }

    co_await fetchBlocks();

    emitHandshake();
}

// ── Fetch BlockedApps property (async) ─────────────────────────────

auto WellbeingManager::fetchBlocks() -> task {
    try {
        // sdbus::Struct (NOT std::tuple) — signature_of adds () struct delimiters.
        // std::tuple's signature_of omits (), producing "astut" (invalid D-Bus).
        using BlockEntry = sdbus::Struct<std::string, uint64_t, uint32_t, uint64_t>;
        using BlockEntries = std::vector<BlockEntry>;

        auto result = co_await m_daemonProxy->callMethodAsync("Get")
                          .onInterface("org.freedesktop.DBus.Properties")
                          .withArguments(wellbeing::DAEMON_INTERFACE, "BlockedApps")
                          .getResultAsAwaitable<sdbus::Variant>();
        auto blocks = result.get<BlockEntries>();

        for (auto &block : blocks) {
            auto &rawAppId = std::get<0>(block);
            auto policyId = std::get<1>(block);
            auto reason = std::get<2>(block);
            auto blockedSince = std::get<3>(block);

            auto appId = AppId::from_raw(rawAppId);
            if (!appId.has_value()) {
                logErr("fetchBlocks: invalid appId '" + rawAppId + "' skipped");
                continue;
            }

            auto br = wellbeing::raw_to_block_reason(reason);
            if (!br.has_value()) {
                logErr("fetchBlocks: invalid BlockReason " + std::to_string(reason) + " skipped");
                continue;
            }

            // Close button is local — no actions vector from daemon.
            m_lockManager->showOverlay(*appId, policyId, *br, blockedSince, {ActionType::Close});
        }
    } catch (const sdbus::Error &e) {
        logInfo("syncBlockedApps: daemon not available (" + std::string(e.what()) + ")");
    }
}

// ── Handshake emit ────────────────────────────────────────────────

void WellbeingManager::emitHandshake() {
    if (g_ctx->focusState.has_value()) {
        emitFocusChanged(g_ctx->focusState);
    } else {
        emitFocusChanged(std::nullopt);
    }
}

// ── BlockedAppsChanged signal subscription ────────────────────────

void WellbeingManager::setupBlockedAppsWatch() {
    if (!m_daemonProxy) {
        logErr("setupBlockedAppsWatch: no daemon proxy");
        return;
    }

    auto &conn = (m_activeBus == DaemonBus::System) ? *m_sysConn : *m_sessConn;

    const auto matchExpr = std::string("type='signal',"
                                       "interface='") +
                           wellbeing::DAEMON_INTERFACE +
                           "',"
                           "member='" +
                           wellbeing::BLOCKED_APPS_CHANGED_SIGNAL +
                           "',"
                           "path='" +
                           wellbeing::DAEMON_OBJECT_PATH + "'";

    try {
        m_blockedAppsSlot = conn.addMatch(
            matchExpr,
            [this](sdbus::Message msg) -> void {
                try {
                    uint32_t uid = 0;
                    std::string rawAppId;
                    bool blocked = false;
                    uint32_t reason = 0;
                    msg >> uid >> rawAppId >> blocked >> reason;

                    auto appId = AppId::from_raw(rawAppId);
                    if (!appId.has_value()) {
                        logErr("setupBlockedAppsWatch: invalid appId '" + rawAppId + "' skipped");
                        return;
                    }

                    if (blocked) {
                        // A new block appeared — re-read the full BlockedApps
                        // property to get policyId and blockedSince.
                        [this]() -> fire_and_forget { co_await fetchBlocks(); }();
                    } else {
                        // Block was removed — hide overlay directly.
                        m_lockManager->hideOverlay(*appId);
                    }
                } catch (const std::exception &e) {
                    logErr("BlockedAppsChanged handler: " + std::string(e.what()));
                } catch (...) {
                    logErr("BlockedAppsChanged handler: unknown exception");
                }
            },
            sdbus::return_slot);

        logInfo("setupBlockedAppsWatch: subscribed to BlockedAppsChanged on " +
                std::string(m_activeBus == DaemonBus::System ? "system" : "session") + " bus");
    } catch (const sdbus::Error &e) {
        logErr("setupBlockedAppsWatch: addMatch failed: " + std::string(e.what()));
    }
}

// ── Daemon bus resolution (4-step) ─────────────────────────────────

auto WellbeingManager::resolveActiveDaemonBus(const std::string &daemonBusName) -> DaemonBus {
    logInfo("resolveActiveDaemonBus: resolving daemon bus (4-step)");

    if (m_sysConn && nameHasOwner(*m_sysConn, daemonBusName)) {
        logInfo("resolveActiveDaemonBus: daemon found on system bus (step 1)");
        return DaemonBus::System;
    }

    if (m_sessConn && nameHasOwner(*m_sessConn, daemonBusName)) {
        logInfo("resolveActiveDaemonBus: daemon found on session bus (step 2)");
        return DaemonBus::Session;
    }

    if (m_sysConn && startServiceByName(*m_sysConn, daemonBusName)) {
        logInfo("resolveActiveDaemonBus: daemon activated on system bus (step 3)");
        return DaemonBus::System;
    }

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

// ── Signal emission ────────────────────────────────────────────────

/// Emit FocusChanged(variant) on BOTH busses.
/// Only emits after `register_plugin` has completed — the first
/// `FocusChanged` after registration serves as the initial focus state.
void WellbeingManager::emitFocusChanged(const std::optional<WindowInfo> &info) {
    if (!m_registered) {
        return; // Gate: wait until register_plugin returns
    }
    bool blocked = info.has_value() && m_lockManager->isOverlayShown(info->appId);
    auto variant = windowInfoToVariant(info, blocked);
    m_sysObject->emitSignal(wellbeing::FOCUS_CHANGED_SIGNAL)
        .onInterface(wellbeing::MANAGER_INTERFACE)
        .withArguments(variant);
    m_sessObject->emitSignal(wellbeing::FOCUS_CHANGED_SIGNAL)
        .onInterface(wellbeing::MANAGER_INTERFACE)
        .withArguments(variant);
}

/// Emit ActivityChanged(tag) on BOTH busses.
void WellbeingManager::emitActivityChanged(FocusActivityTag tag) {
    m_sysObject->emitSignal(wellbeing::ACTIVITY_CHANGED_SIGNAL)
        .onInterface(wellbeing::MANAGER_INTERFACE)
        .withArguments(static_cast<uint32_t>(tag));
    m_sessObject->emitSignal(wellbeing::ACTIVITY_CHANGED_SIGNAL)
        .onInterface(wellbeing::MANAGER_INTERFACE)
        .withArguments(static_cast<uint32_t>(tag));
}

auto WellbeingManager::daemonBusName() -> std::string {
    return wellbeing::DAEMON_INTERFACE; // "org.wellbeing.v1.Controller"
}
