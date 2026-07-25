#pragma once

#include <coroutine>
#include <memory>
#include <optional>
#include <string>

#include <sdbus-c++/sdbus-c++.h>

#include "lockdown.hpp"

// fire_and_forget — eager start, no return value, auto-cleanup on completion.
// Use for top-level entry points (signal handlers, init) where no one awaits.
struct fire_and_forget {
    struct promise_type {
        static auto get_return_object() noexcept -> fire_and_forget { return {}; }
        static auto initial_suspend() noexcept -> std::suspend_never { return {}; }
        static auto final_suspend() noexcept -> std::suspend_never { return {}; }
        void return_void() {}
        static void unhandled_exception() { std::terminate(); }
    };
};

// task — eager start, awaitable from another coroutine, move-only.
// The caller MUST co_await or store it as a member. Dropping a running task
// without awaiting is a programming error (coroutine frame would leak).
// Uses symmetric transfer on final_suspend for safe frame destruction.
struct task {
    struct promise_type {
        std::coroutine_handle<> waiter;

        auto get_return_object() noexcept -> task {
            return task{std::coroutine_handle<promise_type>::from_promise(*this)};
        }
        static auto initial_suspend() noexcept -> std::suspend_never { return {}; }

        struct final_awaiter {
            static auto await_ready() noexcept -> bool { return false; }
            auto await_suspend(std::coroutine_handle<promise_type> h) noexcept -> std::coroutine_handle<> {
                auto w = h.promise().waiter;
                return w ? w : std::noop_coroutine();
            }
            static void await_resume() noexcept {}
        };

        static auto final_suspend() noexcept -> final_awaiter { return {}; }
        void return_void() {}

        static void unhandled_exception() { std::terminate(); }
    };

    std::coroutine_handle<promise_type> m_handle;

    explicit task(std::coroutine_handle<promise_type> h) noexcept : m_handle(h) {}

    task(task &&other) noexcept : m_handle(std::exchange(other.m_handle, nullptr)) {}

    task(const task &) = delete;
    task &operator=(const task &) = delete;

    ~task() {
        if (m_handle) m_handle.destroy();
    }

    struct awaiter {
        std::coroutine_handle<promise_type> m_handle;

        explicit awaiter(std::coroutine_handle<promise_type> h) noexcept : m_handle(h) {}

        auto await_ready() noexcept -> bool { return !m_handle || m_handle.done(); }

        void await_suspend(std::coroutine_handle<> caller) noexcept { m_handle.promise().waiter = caller; }

        void await_resume() noexcept {}
    };

    auto operator co_await() noexcept -> awaiter { return awaiter{m_handle}; }
};

class WellbeingManager {
  public:
    enum class DaemonBus { None, System, Session };

    WellbeingManager(std::shared_ptr<LockManager> lockManager, std::shared_ptr<sdbus::IConnection> sysConnection,
                     std::shared_ptr<sdbus::IConnection> sessConnection);
    ~WellbeingManager();

    // Signal emission (synchronous — D-Bus signals are fire-and-forget)
    void emitFocusChanged(const std::optional<WindowInfo> &info);
    void emitActivityChanged(wellbeing::FocusActivityTag tag);

    /// Called on startup and daemon reconnect.
    auto handshake() -> fire_and_forget;

    /// Shared by handshake and the BlockedAppsChanged signal handler.
    auto fetchBlocks() -> task;

    void onDaemonAppeared();

    auto resolveActiveDaemonBus(const std::string &daemonBusName) -> DaemonBus;
    auto daemonBusName() -> std::string;

    void onDaemonDisappeared();
    void reconnectToDaemon();

  private:
    void emitHandshake();
    void onNameOwnerChanged(const std::string &name, const std::string &oldOwner, const std::string &newOwner,
                            bool isSystem);

    void setupNameOwnerWatch(bool system);
    sdbus::Slot m_sysDaemonWatchSlot;
    sdbus::Slot m_sessDaemonWatchSlot;

    void setupBlockedAppsWatch();
    sdbus::Slot m_blockedAppsSlot;

    std::shared_ptr<sdbus::IProxy> m_daemonProxy;
    std::shared_ptr<sdbus::IConnection> m_sysConn;
    std::shared_ptr<sdbus::IConnection> m_sessConn;
    std::unique_ptr<sdbus::IObject> m_sysObject;
    std::unique_ptr<sdbus::IObject> m_sessObject;
    std::shared_ptr<LockManager> m_lockManager;

    DaemonBus m_activeBus{DaemonBus::None};
    std::string m_daemonBusName;
    bool m_registered{false};
};
