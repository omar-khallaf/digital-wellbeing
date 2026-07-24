#pragma once

#include <coroutine>
#include <memory>
#include <optional>
#include <string>

#include <sdbus-c++/sdbus-c++.h>

#include "lockdown.hpp"

// FireAndForget coroutine return type — async D-Bus operations that don't
// need a result propagated back. Starts immediately, auto-destroys on completion.
struct FireAndForget {
    struct promise_type {
        static auto get_return_object() noexcept -> FireAndForget { return {}; }
        static auto initial_suspend() noexcept -> std::suspend_never { return {}; }
        static auto final_suspend() noexcept -> std::suspend_never { return {}; }
        void return_void() {}
        static void unhandled_exception() { std::terminate(); }
    };
};

// WellbeingManager — Implements org.wellbeing.v1.Manager for overlay/focus IPC.
class WellbeingManager {
  public:
    /// Daemon bus resolution result: which bus the daemon is reachable on.
    enum class DaemonBus { None, System, Session };

    WellbeingManager(std::shared_ptr<LockManager> lockManager, std::shared_ptr<sdbus::IConnection> sysConnection,
                     std::shared_ptr<sdbus::IConnection> sessConnection);
    ~WellbeingManager();

    // Signal emission
    void emitUserAction(const std::string &appId, ActionType action);
    void emitFocusChanged(const std::optional<WindowInfo> &info);
    void emitActivityChanged(wellbeing::FocusActivityTag tag);

    // Async helpers (coroutine-based, non-blocking)
    auto registerWithDaemonAsync() -> FireAndForget;
    auto readActiveBlocksAsync() -> FireAndForget;

    /// Called on daemon bus name appearance — re-registers and re-reads blocks.
    void onDaemonAppeared();

    // Daemon bus resolution
    /// Resolve which bus hosts the daemon via NameHasOwner/StartServiceByName.
    /// Returns DaemonBus::None if all buses fail.
    auto resolveActiveDaemonBus(const std::string &daemonBusName) -> DaemonBus;

    /// Resolve daemon bus name — "org.wellbeing.v1.Controller"
    auto daemonBusName() -> std::string;

    // Cross-bus daemon lifecycle
    /// Called on active daemon disappearance.
    void onDaemonDisappeared();

    /// Re-resolve the daemon and re-establish the proxy.
    void reconnectToDaemon();

  private:
    // NameOwnerChanged callback — isSystem flags which connection fired.
    void onNameOwnerChanged(const std::string &name, const std::string &oldOwner, const std::string &newOwner,
                            bool isSystem);

    // NameOwnerChanged watcher
    void setupNameOwnerWatch(bool system);
    sdbus::Slot m_sysDaemonWatchSlot;
    sdbus::Slot m_sessDaemonWatchSlot;

    // Daemon proxy (for ActiveBlocks reads + BlockStateChanged subscription)
    std::shared_ptr<sdbus::IProxy> m_daemonProxy;

    // Two D-Bus connections (system + session)
    std::shared_ptr<sdbus::IConnection> m_sysConn;
    std::shared_ptr<sdbus::IConnection> m_sessConn;

    // Manager interface objects on both busses
    std::unique_ptr<sdbus::IObject> m_object;     // system bus
    std::unique_ptr<sdbus::IObject> m_sessObject; // session bus

    std::shared_ptr<LockManager> m_lockManager;

    // Active daemon bus tracking
    DaemonBus m_activeBus{DaemonBus::None};

    // Daemon bus name (resolved at construction)
    std::string m_daemonBusName;
};
