#pragma once

#include <coroutine>
#include <memory>
#include <optional>
#include <string>

#include <sdbus-c++/sdbus-c++.h>

#include "lockdown.hpp"

// ── FireAndForget coroutine return type ──────────────────────────────────────
// Used for async D-Bus operations that don't need a result propagated back
// to the caller. The coroutine starts immediately (initial_suspend = never),
// and the frame is destroyed automatically on completion (final_suspend = never).
struct FireAndForget {
    struct promise_type {
        static auto get_return_object() noexcept -> FireAndForget { return {}; }
        static auto initial_suspend() noexcept -> std::suspend_never { return {}; }
        static auto final_suspend() noexcept -> std::suspend_never { return {}; }
        void return_void() {}
        static void unhandled_exception() { std::terminate(); }
    };
};

// WellbeingManager — D-Bus org.wellbeing.v1.Manager interface implementation.
//
// In the declarative architecture the plugin never accepts commands. Instead it:
//   1. Registers with the daemon via RegisterPlugin (reverse discovery).
//   2. Reads ActiveBlocks property for initial block state.
//   3. Subscribes to BlockStateChanged signal for live overlay updates.
//   4. Emits FocusChanged / ActivityChanged / UserAction signals.
//   5. Exposes CurrentFocus property.
//   6. Watches daemon bus name via NameOwnerChanged for auto-recovery.
//
// Now connects to BOTH system and session D-Bus busses simultaneously,
// resolving which one hosts the daemon at runtime.
//
// See docs/architecture/04-plugin-ipc.md, 05-daemon-auth.md, 13-deployment-modes.md.
class WellbeingManager {
  public:
    /// Daemon bus resolution result: which bus the daemon is reachable on.
    enum class DaemonBus { None, System, Session };

    WellbeingManager(std::shared_ptr<LockManager> lockManager, std::shared_ptr<sdbus::IConnection> sysConnection,
                     std::shared_ptr<sdbus::IConnection> sessConnection);
    ~WellbeingManager();

    // ── Reverse discovery ──────────────────────────────────────────────
    void registerWithDaemon();

    // ── Signal emission ────────────────────────────────────────────────
    void emitUserAction(const std::string &appId, ActionType action);
    void emitFocusChanged(const std::optional<WindowInfo> &info);
    void emitActivityChanged(wellbeing::FocusActivityTag tag);

    // ── Daemon state consumption ───────────────────────────────────────
    /// Subscribe to the daemon's BlockStateChanged signal.
    /// Emitted when an app is blocked or unblocked.
    void subscribeToDaemonSignals();

    // ── Async helpers (coroutine-based, non-blocking) ──────────────────
    auto registerWithDaemonAsync() -> FireAndForget;
    auto readActiveBlocksAsync() -> FireAndForget;

    /// Called when the daemon bus name appears (NameOwnerChanged).
    /// Re-registers the plugin and re-reads ActiveBlocks.
    void onDaemonAppeared();

    // ── Daemon bus resolution ─────────────────────────────────────────
    /// 4-step resolution using held connections: NameHasOwner system →
    /// NameHasOwner session → StartServiceByName system →
    /// StartServiceByName session. Returns None if all fail.
    /// @param daemonBusName The well-known bus name to probe (e.g. DAEMON_INTERFACE).
    auto resolveActiveDaemonBus(const std::string &daemonBusName) -> DaemonBus;

    /// Resolve daemon bus name — "org.wellbeing.v1.Controller"
    auto daemonBusName() -> std::string;

    // ── Cross-bus daemon lifecycle ─────────────────────────────────────
    /// Called when the active daemon disappears (NameOwnerChanged
    /// disappearance on the bus we were connected to).
    void onDaemonDisappeared();

    /// Re-resolve the daemon on either bus and re-establish the proxy.
    /// Called after disappearance or appearance on either bus.
    void reconnectToDaemon();

  private:
    // ── NameOwnerChanged callback ──────────────────────────────────────
    /// isSystem indicates which connection fired the match.
    void onNameOwnerChanged(const std::string &name, const std::string &oldOwner, const std::string &newOwner,
                            bool isSystem);

    // ── NameOwnerChanged watcher ───────────────────────────────────────
    /// Register a D-Bus match for NameOwnerChanged on org.freedesktop.DBus
    /// via IConnection::addMatch() on the given bus. When system=true the
    /// watch is set up on m_sysConn (stored in m_sysDaemonWatchSlot);
    /// when false on m_sessConn (stored in m_sessDaemonWatchSlot).
    /// The Slot destructor automatically unsubscribes on destruction.
    void setupNameOwnerWatch(bool system);
    sdbus::Slot m_sysDaemonWatchSlot;
    sdbus::Slot m_sessDaemonWatchSlot;

    // ── Daemon proxy (for ActiveBlocks reads + BlockStateChanged subscription) ─
    std::shared_ptr<sdbus::IProxy> m_daemonProxy;

    // ── Two D-Bus connections (system + session) ───────────────────────
    std::shared_ptr<sdbus::IConnection> m_sysConn;
    std::shared_ptr<sdbus::IConnection> m_sessConn;

    // ── Manager interface objects on BOTH busses ───────────────────────
    std::unique_ptr<sdbus::IObject> m_object;     // system bus
    std::unique_ptr<sdbus::IObject> m_sessObject; // session bus

    std::shared_ptr<LockManager> m_lockManager;

    // ── Active daemon bus tracking ─────────────────────────────────────
    DaemonBus m_activeBus{DaemonBus::None};

    // ── Daemon bus name (resolved at construction) ─────────────────────
    std::string m_daemonBusName;
};
