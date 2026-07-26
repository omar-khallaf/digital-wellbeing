#pragma once

#include <coroutine>
#include <cstdint>
#include <memory>
#include <optional>
#include <string>

#include <sdbus-c++/sdbus-c++.h>

#include "lockdown.hpp"
#include "types.hpp"

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
    auto operator=(const task &) -> task & = delete;

    ~task() {
        if (m_handle) {
            m_handle.destroy();
        }
    }

    struct awaiter {
        std::coroutine_handle<promise_type> m_handle;

        explicit awaiter(std::coroutine_handle<promise_type> h) noexcept : m_handle(h) {}

        [[nodiscard]] auto await_ready() const noexcept -> bool { return !m_handle || m_handle.done(); }

        void await_suspend(std::coroutine_handle<> caller) const noexcept { m_handle.promise().waiter = caller; }

        void await_resume() noexcept {}
    };

    auto operator co_await() const noexcept -> awaiter { return awaiter{m_handle}; }
};

namespace wellbeing {

class WellbeingManager {
  public:
    enum class DaemonBus : std::uint8_t { None, System, Session };

    WellbeingManager(std::shared_ptr<LockManager> lockManager, std::shared_ptr<sdbus::IConnection> sysConnection,
                     std::shared_ptr<sdbus::IConnection> sessConnection);
    ~WellbeingManager();

    /// Emit the unified Event signal with focus-state info.
    /// For Focus/Block, use the WindowInfo overload.  For tag-only events
    /// (Idle, Resume, LogOut, Unfocus), use the tag-only overload.
    void emitEvent(EventTag tag, const std::string &app_class, const std::string &title, uint32_t pid,
                   uint32_t power_tag);

    void emitFocusEvent(const std::optional<WindowInfo> &info);

    /// Convenience: emit a tag-only event (Idle, Resume, LogOut, Unfocus, Locked).
    void emitSimpleEvent(EventTag tag);

    /// Called on startup and daemon reconnect.
    auto handshake() -> fire_and_forget;

    /// Shared by handshake and the BlockedAppsChanged signal handler.
    auto fetchBlocks() -> task;

    void onDaemonAppeared();

    auto daemonBusName() -> std::string;

    void onDaemonDisappeared();
    void reconnectToDaemon();

  private:
    void onNameOwnerChanged(const std::string &name, const std::string &oldOwner, const std::string &newOwner,
                            bool isSystem);

    void setupNameOwnerWatch(bool system);
    sdbus::Slot m_sysDaemonWatchSlot;
    sdbus::Slot m_sessDaemonWatchSlot;

    void setupBlockedAppsWatch();
    sdbus::Slot m_blockedAppsSlot;

    void setupSystemWatchers();
    void handlePrepareForSleep(bool sleeping);
    void handlePrepareForShutdown(bool shuttingDown);
    void handleScreenSaverActive(bool active);
    sdbus::Slot m_logindSlot;
    sdbus::Slot m_screenSaverSlot;

    std::shared_ptr<sdbus::IProxy> m_daemonProxy;
    std::shared_ptr<sdbus::IConnection> m_sysConn;
    std::shared_ptr<sdbus::IConnection> m_sessConn;
    std::unique_ptr<sdbus::IObject> m_sysObject;
    std::unique_ptr<sdbus::IObject> m_sessObject;
    std::shared_ptr<LockManager> m_lockManager;

    DaemonBus m_activeBus{DaemonBus::None};
    std::string m_daemonBusName;
    bool m_registered{false};
    bool m_screenLocked{false};
};

} // namespace wellbeing
