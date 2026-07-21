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
// D-Bus calls to the daemon use C++20 coroutines (co_await) via
// sdbus-c++'s getResultAsAwaitable() API, driven by the event loop
// running in a dedicated std::jthread.
//
// See docs/architecture/04-plugin-ipc.md and 05-daemon-auth.md.
// =============================================================================

#include "wellbeing_manager.hpp"

#include <cstdint>
#include <memory>
#include <optional>
#include <string>
#include <tuple>
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
    return sdbus::Variant{std::tuple{
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
/// windowInfoToVariant(currentFocus): no focus → Desktop (tag 1); focused app
/// → App (tag 2) with overlay_shown reflecting whether that app is blocked.
auto buildCurrentFocusVariant() -> sdbus::Variant { return windowInfoToVariant(g_ctx->currentFocus); }

} // anonymous namespace

// =============================================================================
// Daemon bus resolution (4-step)
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

/// Create an ephemeral (no well-known name) bus connection for probing.
auto createProbeConnection(bool system) -> std::shared_ptr<sdbus::IConnection> {
    try {
        auto conn = system ? sdbus::createSystemBusConnection() : sdbus::createSessionBusConnection();
        return std::shared_ptr<sdbus::IConnection>(conn.release());
    } catch (const sdbus::Error &) {
        return nullptr;
    }
}

} // anonymous namespace

auto WellbeingManager::resolveDaemonBus() -> std::optional<WellbeingManager::BusVariant> {
    const auto DAEMON_NAME = daemonBusName();

    // Probe connection for the system bus.
    auto sysConn = createProbeConnection(true);

    // 1. System bus already has the daemon?
    if (sysConn && nameHasOwner(*sysConn, DAEMON_NAME)) {
        return BusVariant::System;
    }

    // Probe connection for the session bus.
    auto sessConn = createProbeConnection(false);

    // 2. Session bus already has the daemon?
    if (sessConn && nameHasOwner(*sessConn, DAEMON_NAME)) {
        return BusVariant::Session;
    }

    // 3. Activate the SYSTEM daemon.
    if (sysConn && startServiceByName(*sysConn, DAEMON_NAME)) {
        return BusVariant::System;
    }

    // 4. Activate the SESSION daemon.
    if (sessConn && startServiceByName(*sessConn, DAEMON_NAME)) {
        return BusVariant::Session;
    }

    return std::nullopt; // all four steps failed — degraded mode
}

// =============================================================================
// WellbeingManager
// =============================================================================

WellbeingManager::WellbeingManager(std::shared_ptr<LockManager> lockManager,
                                   std::shared_ptr<sdbus::IConnection> connection)
    : m_conn(std::move(connection)),
      m_object(sdbus::createObject(*m_conn, sdbus::ObjectPath{wellbeing::MANAGER_OBJECT_PATH})),
      m_lockManager(std::move(lockManager)), m_daemonBusName(daemonBusName()) {
    m_object
        ->addVTable(sdbus::registerProperty(wellbeing::CURRENT_FOCUS_PROPERTY).withGetter([]() -> sdbus::Variant {
            return buildCurrentFocusVariant();
        }),
                    sdbus::registerSignal(wellbeing::USER_ACTION_SIGNAL)
                        .withParameters<std::string, uint32_t>({"app_id", "action"}),
                    sdbus::registerSignal(wellbeing::FOCUS_CHANGED_SIGNAL).withParameters<sdbus::Variant>({"window"}),
                    sdbus::registerSignal(wellbeing::ACTIVITY_CHANGED_SIGNAL).withParameters<uint32_t>({"activity"}))
        .forInterface(wellbeing::MANAGER_INTERFACE);

    // Wire LockManager button clicks → our emitUserAction.
    m_lockManager->setUserActionCallback(
        [this](const AppId &appId, ActionType action) -> void { emitUserAction(appId.value(), action); });

    // ── Create daemon proxy for ActiveBlocks reads + signal subscription ──
    try {
        m_daemonProxy = sdbus::createProxy(*m_conn, sdbus::ServiceName{m_daemonBusName},
                                           sdbus::ObjectPath{wellbeing::DAEMON_OBJECT_PATH});
    } catch (const sdbus::Error &e) {
        logErr("WellbeingManager: failed to create daemon proxy: " + std::string(e.what()));
    }

    // ── Start event loop ──────────────────────────────────────────────────
    // Drives async D-Bus coroutine resumptions and signal delivery.
    // Runs in a background thread managed by sdbus-c++ internally.
    try {
        m_conn->enterEventLoopAsync();
    } catch (const sdbus::Error &e) {
        logErr("WellbeingManager: failed to start event loop: " + std::string(e.what()));
    }

    // ── NameOwnerChanged watcher (via addMatch) ─────────────────────────
    // Subscribe to org.freedesktop.DBus.NameOwnerChanged to detect when
    // the daemon bus name appears (or restarts) and auto-recover.
    // Uses IConnection::addMatch() with a D-Bus match expression.
    setupNameOwnerWatch();

    // ── Reverse-discovery + initial state sync ─────────────────────────
    registerWithDaemon();

    // ── Synchronise local overlay state from daemon's ActiveBlocks ─────
    readActiveBlocks();

    // ── Subscribe to daemon block state signals ────────────────────────
    subscribeToDaemonSignals();
}

WellbeingManager::~WellbeingManager() {
    // m_daemonWatchSlot (sdbus::Slot) is destroyed automatically as a member,
    // which unsubscribes the NameOwnerChanged match.
    // Stop the internal event loop thread.
    m_conn->leaveEventLoop();
}

// ── Reverse discovery ──────────────────────────────────────────────

void WellbeingManager::registerWithDaemon() {
    if (!m_daemonProxy) {
        logErr("registerWithDaemon: no daemon proxy available");
        return;
    }
    try {
        m_daemonProxy->callMethod(wellbeing::REGISTER_PLUGIN_METHOD).onInterface(wellbeing::DAEMON_INTERFACE);
        logInfo("registerWithDaemon: registered plugin instance");
    } catch (const sdbus::Error &e) {
        logInfo("registerWithDaemon: daemon not reachable (" + std::string(e.what()) +
                ") — will be discovered via NameOwnerChanged");
    }
}

// ── Async registration (coroutine-based) ───────────────────────────

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

// ── Daemon state consumption (declarative) ─────────────────────────

void WellbeingManager::readActiveBlocks() {
    if (!m_daemonProxy) {
        logErr("readActiveBlocks: no daemon proxy");
        return;
    }

    try {
        // ActiveBlocks is a property on the daemon returning Vec<ActiveBlockEntry>.
        // Each entry: {app_id, policy_id, reason, blocked_since, available_actions}
        // D-Bus signature: a(s(tutau))
        std::vector<std::tuple<std::string, uint64_t, uint32_t, uint64_t, std::vector<uint32_t>>> blocks;
        m_daemonProxy->callMethod(wellbeing::GET_PROPERTY_METHOD)
            .onInterface(wellbeing::PROPERTIES_INTERFACE)
            .withArguments(wellbeing::DAEMON_INTERFACE, "ActiveBlocks")
            .storeResultsTo(blocks);

        for (auto &block : blocks) {
            auto &rawAppId = std::get<0>(block);
            auto policyId = std::get<1>(block);
            auto reason = std::get<2>(block);
            auto blockedSince = std::get<3>(block);
            auto &actions = std::get<4>(block);

            auto appId = AppId::from_raw(rawAppId);
            if (!appId.has_value()) {
                logErr("readActiveBlocks: invalid appId '" + rawAppId + "' skipped");
                continue;
            }

            auto br = wellbeing::raw_to_block_reason(reason);
            if (!br.has_value()) {
                logErr("readActiveBlocks: invalid BlockReason " + std::to_string(reason) + " skipped");
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

        logInfo("readActiveBlocks: synced " + std::to_string(blocks.size()) + " active blocks");
    } catch (const sdbus::Error &e) {
        logInfo("readActiveBlocks: daemon not available yet (" + std::string(e.what()) + ")");
    }
}

// ── Async readActiveBlocks (coroutine-based) ───────────────────────

auto WellbeingManager::readActiveBlocksAsync() -> FireAndForget {
    if (!m_daemonProxy) {
        logErr("readActiveBlocksAsync: no daemon proxy");
        co_return;
    }

    try {
        using BlockTuple = std::tuple<std::string, uint64_t, uint32_t, uint64_t, std::vector<uint32_t>>;
        std::vector<BlockTuple> blocks;
        blocks = co_await m_daemonProxy->callMethodAsync(wellbeing::GET_PROPERTY_METHOD)
                     .onInterface(wellbeing::PROPERTIES_INTERFACE)
                     .withArguments(wellbeing::DAEMON_INTERFACE, "ActiveBlocks")
                     .getResultAsAwaitable<decltype(blocks)>();

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

        logInfo("readActiveBlocksAsync: synced " + std::to_string(blocks.size()) + " active blocks");
    } catch (const sdbus::Error &e) {
        logInfo("readActiveBlocksAsync: daemon not available yet (" + std::string(e.what()) + ")");
    }
}

/// Subscribe to the daemon's BlockStateChanged signal.
///
/// BlockStateChanged signature: (uid: u32, app_id: s, blocked: b, reason: u32)
/// We subscribe via PropertiesChanged on org.wellbeing.v1.Controller or via
/// a dedicated signal match. sdbus-c++ doesn't natively support signal
/// subscription on a proxy, so we use the connection-level sd-bus match.
void WellbeingManager::subscribeToDaemonSignals() {
    if (!m_daemonProxy) {
        logErr("subscribeToDaemonSignals: no daemon proxy");
        return;
    }

    try {
        logInfo("subscribeToDaemonSignals: ActiveBlocks sync registered "
                "(full signal subscription requires sdbus-c++ signal API)");

        // TODO: In a future iteration, use the native sd-bus API directly:
        //   sd_bus_match_signal(m_conn->get(), nullptr, DAEMON_INTERFACE,
        //                       DAEMON_OBJECT_PATH, DAEMON_INTERFACE,
        //                       "BlockStateChanged", onBlockStateChanged, nullptr);
        // For now, readActiveBlocks() is called on focus changes and periodically.
    } catch (const sdbus::Error &e) {
        logErr("subscribeToDaemonSignals: failed: " + std::string(e.what()));
    }
}

// ── NameOwnerChanged match setup ────────────────────────────────────

void WellbeingManager::setupNameOwnerWatch() {
    const auto matchExpr = std::string("type='signal',"
                                       "sender='org.freedesktop.DBus',"
                                       "interface='org.freedesktop.DBus',"
                                       "member='NameOwnerChanged',"
                                       "path='/org/freedesktop/DBus'");

    try {
        m_daemonWatchSlot = m_conn->addMatch(
            matchExpr,
            [this](sdbus::Message msg) -> void {
                std::string name;
                std::string oldOwner;
                std::string newOwner;
                msg >> name >> oldOwner >> newOwner;
                onNameOwnerChanged(name, oldOwner, newOwner);
            },
            sdbus::return_slot);

        logInfo("setupNameOwnerWatch: watching NameOwnerChanged for " + m_daemonBusName);
    } catch (const sdbus::Error &e) {
        logErr("setupNameOwnerWatch: addMatch failed: " + std::string(e.what()));
    }
}

// ── Daemon recovery via NameOwnerChanged ────────────────────────────

void WellbeingManager::onDaemonAppeared() {
    logInfo("onDaemonAppeared: daemon bus name appeared — re-registering and syncing state");

    // Re-create the daemon proxy (the old one may be stale).
    try {
        m_daemonProxy = sdbus::createProxy(*m_conn, sdbus::ServiceName{m_daemonBusName},
                                           sdbus::ObjectPath{wellbeing::DAEMON_OBJECT_PATH});
    } catch (const sdbus::Error &e) {
        logErr("onDaemonAppeared: failed to create daemon proxy: " + std::string(e.what()));
        return;
    }

    // Re-register plugin instance — the daemon may have restarted.
    registerWithDaemon();

    // Re-read all active blocks to synchronise overlay state.
    readActiveBlocks();

    // Re-subscribe to daemon signals.
    subscribeToDaemonSignals();
}

void WellbeingManager::onNameOwnerChanged(const std::string &name, const std::string &oldOwner,
                                          const std::string &newOwner) {
    if (name != m_daemonBusName) {
        return; // not our daemon
    }

    if (!oldOwner.empty() && newOwner.empty()) {
        logInfo("onNameOwnerChanged: daemon '" + name + "' disappeared");
    } else if (oldOwner.empty() && !newOwner.empty()) {
        logInfo("onNameOwnerChanged: daemon '" + name + "' appeared");
        onDaemonAppeared();
    } else if (!oldOwner.empty() && !newOwner.empty()) {
        logInfo("onNameOwnerChanged: daemon '" + name + "' changed owner: " + oldOwner + " → " + newOwner);
        onDaemonAppeared();
    }
}

// ── Signal emission ────────────────────────────────────────────────

/// Emit UserAction(app_id, action). The daemon looks up the corresponding
/// policy_id from its own ActiveBlocks state — no echo token needed.
void WellbeingManager::emitUserAction(const std::string &appId, ActionType action) {
    m_object->emitSignal(wellbeing::USER_ACTION_SIGNAL)
        .onInterface(wellbeing::MANAGER_INTERFACE)
        .withArguments(appId, static_cast<uint32_t>(action));
}

/// Emit FocusChanged(variant) reflecting the current focused window.
void WellbeingManager::emitFocusChanged(const std::optional<WindowInfo> &info) {
    m_object->emitSignal(wellbeing::FOCUS_CHANGED_SIGNAL)
        .onInterface(wellbeing::MANAGER_INTERFACE)
        .withArguments(windowInfoToVariant(info));
}

/// Emit ActivityChanged(tag) when user activity state changes.
/// Uses FocusActivityTag enum: Idle=0, Resumed=1 (instead of old bool).
void WellbeingManager::emitActivityChanged(FocusActivityTag tag) {
    m_object->emitSignal(wellbeing::ACTIVITY_CHANGED_SIGNAL)
        .onInterface(wellbeing::MANAGER_INTERFACE)
        .withArguments(static_cast<uint32_t>(tag));
}

auto WellbeingManager::daemonBusName() -> std::string {
    return wellbeing::DAEMON_INTERFACE; // "org.wellbeing.v1.Controller"
}
