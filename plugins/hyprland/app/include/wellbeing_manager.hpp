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
// See docs/architecture/04-plugin-ipc.md, 05-daemon-auth.md, 13-deployment-modes.md.
class WellbeingManager {
  public:
    WellbeingManager(std::shared_ptr<LockManager> lockManager, std::shared_ptr<sdbus::IConnection> connection);
    ~WellbeingManager();

    // ── Reverse discovery ──────────────────────────────────────────────
    void registerWithDaemon();

    // ── Signal emission ────────────────────────────────────────────────
    void emitUserAction(const std::string &appId, ActionType action);
    void emitFocusChanged(const std::optional<WindowInfo> &info);
    void emitActivityChanged(wellbeing::FocusActivityTag tag);

    // ── Daemon state consumption (declarative) ─────────────────────────
    /// Read the daemon's ActiveBlocks property and synchronise local overlays.
    /// Called once on startup and after reconnection.
    void readActiveBlocks();

    /// Subscribe to the daemon's BlockStateChanged signal.
    /// Emitted when an app is blocked or unblocked.
    void subscribeToDaemonSignals();

    /// Called when the daemon bus name appears (NameOwnerChanged).
    /// Re-registers the plugin and re-reads ActiveBlocks.
    void onDaemonAppeared();

    // ── Daemon bus resolution ─────────────────────────────────────────
    /// 4-step resolution: system present → session present → activate system
    /// → activate session. Returns system bus or session bus, std::nullopt
    /// if unreachable. See 13-deployment-modes.md §Plugin Resolution.
    enum class BusVariant { System, Session };
    static auto resolveDaemonBus() -> std::optional<BusVariant>;

    /// Resolve daemon bus name — "org.wellbeing.v1.Controller"
    static auto daemonBusName() -> std::string;

  private:
    // ── Async helpers (coroutine-based) ────────────────────────────────
    auto registerWithDaemonAsync() -> FireAndForget;
    auto readActiveBlocksAsync() -> FireAndForget;

    // ── NameOwnerChanged callback ──────────────────────────────────────
    void onNameOwnerChanged(const std::string &name, const std::string &oldOwner, const std::string &newOwner);

    // ── NameOwnerChanged watcher ───────────────────────────────────────
    /// Register a D-Bus match for NameOwnerChanged on org.freedesktop.DBus
    /// via IConnection::addMatch(), so we detect when the daemon appears
    /// (or restarts) at any point after plugin init. The Slot destructor
    /// automatically unsubscribes when WellbeingManager is destroyed.
    void setupNameOwnerWatch();
    sdbus::Slot m_daemonWatchSlot;

    // ── Daemon proxy (for ActiveBlocks reads + BlockStateChanged subscription) ─
    std::shared_ptr<sdbus::IProxy> m_daemonProxy;

    std::shared_ptr<sdbus::IConnection> m_conn;
    std::unique_ptr<sdbus::IObject> m_object;
    std::shared_ptr<LockManager> m_lockManager;

    // ── Daemon bus name (resolved at construction) ─────────────────────
    std::string m_daemonBusName;
};
