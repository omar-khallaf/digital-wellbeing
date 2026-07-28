// =============================================================================
// Plugin entry point — PLUGIN_INIT / PLUGIN_EXIT.
//
// Creates PluginState with lock-free SPSC channels, starts the D-Bus thread,
// sets up the wl_event_loop callback for compositor-targeted commands, and
// installs all hooks.
// =============================================================================

#include <memory>
#include <stdexcept>
#include <string>

#include <sys/eventfd.h>
#include <unistd.h>

// Hyprland plugin API
#include <hyprland/Compositor.hpp>
#include <hyprland/event/EventBus.hpp>
#include <hyprland/plugins/PluginAPI.hpp>

#include "hooks.hpp"
#include "logging.hpp"
#include "messages.hpp"
#include "plugin_state.hpp"

using wellbeing::DbusThread;
using wellbeing::IdleState;
using wellbeing::IdleTracker;
using wellbeing::logErr;
using wellbeing::logInfo;

inline HANDLE PHANDLE = nullptr;

// ── wl_event_loop callback for chan B (D-Bus → compositor) ─────────────────

/// Push a message to the D-Bus thread via chan C.
template<typename T>
static void pushToDbus(T &&msg) {
    if (!wellbeing::g_ps || !wellbeing::g_ps->channels) {
        return;
    }
    auto ws = wellbeing::g_ps->channels->msgQueue.prepare_write(1);
    if (ws.get_items_written() > 0) {
        for (auto &slot : ws) {
            slot = std::forward<T>(msg);
        }
        eventfd_write(wellbeing::g_ps->channels->msgEfd, 1);
    }
}

static auto onCmdEventFd(int fd, uint32_t mask, void *data) -> int {
    if ((mask & WL_EVENT_READABLE) == 0U) {
        return 0;
    }

    eventfd_t val = 0;
    eventfd_read(fd, &val); // drain the eventfd

    auto *channels = static_cast<wellbeing::ThreadChannels *>(data);
    if (channels == nullptr) {
        return 0;
    }

    auto &cmdQueue = channels->cmdQueue;
    const size_t available = cmdQueue.get_num_items_ready();
    if (available == 0) {
        return 0;
    }

    auto scope = cmdQueue.prepare_read(available);
    for (const auto &item : scope) {
        if ((wellbeing::g_ps == nullptr) || !wellbeing::g_ps->lockManager) {
            continue;
        }

        std::visit(wellbeing::Overloaded{
                       [](const wellbeing::BlockCmd &cmd) -> void {
                           wellbeing::g_ps->lockManager->apply(cmd);
                           const auto focused = wellbeing::focusedWindowClass();
                           // if the current window is the blocked one send a
                           // block event to terminate the focus interval
                           if (focused.empty() || focused != cmd.wclass) {
                               return;
                           }
                           pushToDbus(wellbeing::BlockedFocus{
                               .wclass = cmd.wclass,
                               .wTitle = wellbeing::focusedWindowTitle(),
                           });
                       },
                       [](const wellbeing::UnblockCmd &cmd) -> void {
                           wellbeing::g_ps->lockManager->apply(cmd);
                           const auto focused = wellbeing::focusedWindowClass();
                           if (focused.empty() || focused != cmd.wclass) {
                               return;
                           }
                           // if the current window is the unblocked one send a
                           // focus event to open a focus interval
                           pushToDbus(wellbeing::FocusUpdate{
                               .wclass = cmd.wclass,
                               .wTitle = wellbeing::focusedWindowTitle(),
                           });
                       },
                       [](const wellbeing::SyncAllCmd &cmd) -> void {
                           // SyncAllCmd replaces all blocked state — only emit
                           // a focus signal if the focused window's blocked
                           // status actually changed.
                           const auto focused = wellbeing::focusedWindowClass();
                           if (focused.empty()) {
                               wellbeing::g_ps->lockManager->apply(cmd);
                               return;
                           }
                           const bool wasBlocked = wellbeing::g_ps->lockManager->isBlocked(focused);
                           wellbeing::g_ps->lockManager->apply(cmd);
                           const bool nowBlocked = wellbeing::g_ps->lockManager->isBlocked(focused);
                           if (wasBlocked != nowBlocked) {
                               if (nowBlocked) {
                                   pushToDbus(wellbeing::BlockedFocus{
                                       .wclass = focused,
                                       .wTitle = wellbeing::focusedWindowTitle(),
                                   });
                               } else {
                                   pushToDbus(wellbeing::FocusUpdate{
                                       .wclass = focused,
                                       .wTitle = wellbeing::focusedWindowTitle(),
                                   });
                               }
                           }
                       },
                   },
                   item);
    }
    // scope commits on destruction

    return 0;
}

// ═════════════════════════════════════════════════════════════════════════════

extern "C" APICALL EXPORT std::string PLUGIN_API_VERSION() { return HYPRLAND_API_VERSION; }

extern "C" APICALL EXPORT PLUGIN_DESCRIPTION_INFO PLUGIN_INIT(HANDLE handle) {
    PHANDLE = handle;

    {
        const std::string hash = __hyprland_api_get_hash();
        const std::string client_hash = __hyprland_api_get_client_hash();

        if (hash != client_hash) {
            HyprlandAPI::addNotification(PHANDLE, "[wellbeing-lockdown] Version mismatch",
                                         CHyprColor{1.0, 0.2F, 0.2F, 1.0}, 5000);
            logErr("version mismatch: headers '" + client_hash + "' != compositor '" + hash + "'");
            throw std::runtime_error("version mismatch");
        }
    }

    auto state = std::make_unique<wellbeing::PluginState>();

    state->lockManager = std::make_unique<wellbeing::LockManager>();
    state->channels = std::make_unique<wellbeing::ThreadChannels>();

    // Create eventfds
    state->channels->cmdEfd = eventfd(0, EFD_CLOEXEC | EFD_NONBLOCK);
    state->channels->msgEfd = eventfd(0, EFD_CLOEXEC | EFD_NONBLOCK);
    state->channels->ackEfd = eventfd(0, EFD_CLOEXEC | EFD_NONBLOCK);

    if (state->channels->cmdEfd < 0 || state->channels->msgEfd < 0 || state->channels->ackEfd < 0) {
        logErr("PLUGIN_INIT: eventfd creation failed");
        return PLUGIN_DESCRIPTION_INFO{"", "", "", ""};
    }

    // Register wl_event_loop callback for chan B (D-Bus → compositor commands)
    auto *eventSrc = wl_event_loop_add_fd(g_pCompositor->m_wlEventLoop, state->channels->cmdEfd, WL_EVENT_READABLE,
                                          onCmdEventFd, state->channels.get());
    if (eventSrc == nullptr) {
        logErr("PLUGIN_INIT: wl_event_loop_add_fd failed");
        return PLUGIN_DESCRIPTION_INFO{"", "", "", ""};
    }
    // The source is automatically cleaned up by the event loop; we don't
    // need to track it explicitly since it lives for the compositor's lifetime.

    // Start the D-Bus thread
    state->dbusThread = std::make_unique<DbusThread>(*state->channels);

    // Idle tracker with transition callback
    {
        auto onTransition = [](IdleState newState) -> void {
            if (!wellbeing::g_ps || !wellbeing::g_ps->channels) {
                return;
            }

            wellbeing::IdleChanged msg{newState == IdleState::Idle};
            auto ws = wellbeing::g_ps->channels->msgQueue.prepare_write(1);
            if (ws.get_items_written() > 0) {
                for (auto &slot : ws) {
                    slot = msg;
                }
                eventfd_write(wellbeing::g_ps->channels->msgEfd, 1);
            }
        };

        state->idleTracker =
            std::make_unique<IdleTracker>(std::move(onTransition), wellbeing::focusedWindowHasIdleInhibitor);
    }

    // Install global singleton before hooks fire
    wellbeing::g_ps = std::move(state);

    wellbeing::registerHooks();

    logInfo("PLUGIN_INIT: complete");

    return PLUGIN_DESCRIPTION_INFO{
        "wellbeing-lockdown",
        "Digital Wellbeing — compositor plugin for screen-time management",
        "Digital Wellbeing Authors",
        "0.2.0",
    };
}

extern "C" APICALL EXPORT void PLUGIN_EXIT() {
    logInfo("PLUGIN_EXIT: shutting down");

    if (wellbeing::g_ps == nullptr) {
        return;
    }

    // Send ShutdownMsg to the D-Bus thread via chan C
    if (wellbeing::g_ps->channels) {
        wellbeing::ShutdownMsg shutdown;
        auto ws = wellbeing::g_ps->channels->msgQueue.prepare_write(1);
        if (ws.get_items_written() > 0) {
            for (auto &slot : ws) {
                slot = shutdown;
            }
            eventfd_write(wellbeing::g_ps->channels->msgEfd, 1);
        }
    }

    // Join and destroy the D-Bus thread
    if (wellbeing::g_ps->dbusThread != nullptr) {
        wellbeing::g_ps->dbusThread->requestShutdown();
        // The eventfd write above ensures the thread wakes and processes
        // the ShutdownMsg, which triggers loop exit.
        wellbeing::g_ps->dbusThread->join();
        wellbeing::g_ps->dbusThread.reset();
    }

    // Close eventfds
    if (wellbeing::g_ps->channels) {
        if (wellbeing::g_ps->channels->cmdEfd >= 0) {
            close(wellbeing::g_ps->channels->cmdEfd);
        }
        if (wellbeing::g_ps->channels->msgEfd >= 0) {
            close(wellbeing::g_ps->channels->msgEfd);
        }
        if (wellbeing::g_ps->channels->ackEfd >= 0) {
            close(wellbeing::g_ps->channels->ackEfd);
        }
    }

    wellbeing::g_ps.reset();
    PHANDLE = nullptr;

    logInfo("PLUGIN_EXIT: complete");
}
