// =============================================================================
// wellbeing-lockdown — Hyprland compositor plugin
//
// Connects to BOTH system and session D-Bus busses simultaneously (no
// probing, no background retry thread). The daemon bus is resolved via
// resolveActiveDaemonBus() (4-step: NameHasOwner system → NameHasOwner
// session → StartServiceByName system → StartServiceByName session).
// NameOwnerChanged watches on both busses detect daemon (re)appearance
// and trigger auto-recovery across busses.
//
// Provides:
//   - FocusChanged    signal   (Option<WindowInfo> on every focus switch)
//   - ActivityChanged signal   (FocusActivityTag: idle / resumed)
//   - UserAction      signal   (button click — app_id + action only)
//   - CurrentFocus    property (read-only FocusVariant)
//
// Uses declarative block state: reads the daemon's ActiveBlocks property
// and subscribes to BlockStateChanged signal — never receives commands.
//
// Single source of truth for focus: g_ctx->currentFocus is the only focus
// state. LockManager queries it via getFocusedApp() instead of receiving
// duplicate setFocusedApp calls.
//
// See docs/architecture/04-plugin-ipc.md and 05-daemon-auth.md.
// =============================================================================

#include <memory>
#include <optional>
#include <stdexcept>
#include <string>

#include <unistd.h> // getuid

// Hyprland plugin API (headers fetched by the superbuild into staging/include)
#include <hyprland/Compositor.hpp>
#include <hyprland/desktop/view/Window.hpp>         // CWindow (m_initialClass, m_class, m_title, getPID)
#include <hyprland/event/EventBus.hpp>              // Event::bus()
#include <hyprland/managers/input/InputManager.hpp> // g_pInputManager, getMouseCoordsInternal()
#include <hyprland/plugins/PluginAPI.hpp>
#include <hyprland/render/OpenGL.hpp>
#include <sdbus-c++/sdbus-c++.h>

#include "lockdown.hpp"
#include "logging.hpp"
#include "plugin_state.hpp"
#include "types.hpp"
#include "wellbeing_manager.hpp"

using wellbeing::AppId;
using wellbeing::FocusActivityTag;
using wellbeing::g_ctx;
using wellbeing::IdleState;
using wellbeing::IdleTracker;
using wellbeing::logErr;
using wellbeing::logInfo;

// Handle returned by PLUGIN_INIT; required by Hyprland plugin API.
inline HANDLE PHANDLE = nullptr;

// =============================================================================
// Event::bus() listeners — registered once in PLUGIN_INIT
// =============================================================================

namespace {

// ── Activity tracking ────────────────────────────────────────────
// Delegated entirely to IdleTracker (plugin_state.hpp / idle_tracker.*).
// Input hooks call notifyActivity(); the render hook calls tick().
// On state transitions, IdleTracker fires its injected callback which
// emits the D-Bus ActivityChanged signal and logs the transition.

/// Returns true when the focused Hyprland window has an active Wayland
/// idle-inhibit protocol inhibitor (zwp_idle_inhibitor_v1).
/// Uses the weak ref stored in PluginState::focusedWindow (set on every
/// focus switch by WINDOW_FOCUS_HOOK) — no window iteration needed.
/// Delegates to CInputManager::isWindowInhibiting() which checks both
/// the idle-inhibit protocol and shell surface constraints.
auto focusedWindowHasIdleInhibitor() -> bool {
    if (!g_pInputManager) {
        return false;
    }
    const auto window = g_ctx->focusedWindow.lock();
    if (!window) {
        return false;
    }
    return g_pInputManager->isWindowInhibiting(window, false);
}

void registerRenderHook() {
    static auto HOOK = Event::bus()->m_events.render.stage.listen([](eRenderStage stage) -> void {
        try {
            if (stage == eRenderStage::RENDER_POST_WINDOW) {
                g_ctx->lockManager->drawOverlay();
            }

            if (stage == eRenderStage::RENDER_POST) {
                g_ctx->idleTracker->tick();
            }
        } catch (const std::exception &e) {
            logErr("render hook: " + std::string(e.what()));
        } catch (...) {
            logErr("render hook: unknown exception");
        }
    });
    (void)HOOK;
}

void registerInputHooks() {
    // Mouse button → user activity + overlay input trapping
    // Coordinates come from g_pInputManager because SButtonEvent has no
    // position field (only button + state).
    static auto MOUSE_HOOK = Event::bus()->m_events.input.mouse.button.listen(
        [](IPointer::SButtonEvent, Event::SCallbackInfo &info) -> void {
            try {
                g_ctx->idleTracker->notifyActivity();
                const auto coords = g_pInputManager->getMouseCoordsInternal();
                if (g_ctx->lockManager->onMouseClick(static_cast<double>(coords.x), static_cast<double>(coords.y))) {
                    info.cancelled = true;
                }
            } catch (const std::exception &e) {
                logErr("mouse click: " + std::string(e.what()));
            } catch (...) {
                logErr("mouse click: unknown exception");
            }
        });

    // Mouse motion → user activity only (no trapping)
    static auto MOUSE_MOVE_HOOK =
        Event::bus()->m_events.input.mouse.move.listen([](const Vector2D &, Event::SCallbackInfo &) -> void {
            try {
                g_ctx->idleTracker->notifyActivity();
            } catch (const std::exception &e) {
                logErr("mouse move: " + std::string(e.what()));
            } catch (...) {
                logErr("mouse move: unknown exception");
            }
        });

    // Keyboard key → user activity + overlay input trapping
    static auto KEY_HOOK =
        Event::bus()->m_events.input.keyboard.key.listen([](IKeyboard::SKeyEvent, Event::SCallbackInfo &info) -> void {
            try {
                g_ctx->idleTracker->notifyActivity();
                if (g_ctx->lockManager->onKey()) {
                    info.cancelled = true;
                }
            } catch (const std::exception &e) {
                logErr("keyboard: " + std::string(e.what()));
            } catch (...) {
                logErr("keyboard: unknown exception");
            }
        });

    // Touch events → user activity
    static auto TOUCH_DOWN_HOOK =
        Event::bus()->m_events.input.touch.down.listen([](const ITouch::SDownEvent &, Event::SCallbackInfo &) -> void {
            try {
                g_ctx->idleTracker->notifyActivity();
            } catch (const std::exception &e) {
                logErr("touch down: " + std::string(e.what()));
            } catch (...) {
                logErr("touch down: unknown exception");
            }
        });

    static auto TOUCH_UP_HOOK =
        Event::bus()->m_events.input.touch.up.listen([](const ITouch::SUpEvent &, Event::SCallbackInfo &) -> void {
            try {
                g_ctx->idleTracker->notifyActivity();
            } catch (const std::exception &e) {
                logErr("touch up: " + std::string(e.what()));
            } catch (...) {
                logErr("touch up: unknown exception");
            }
        });

    static auto TOUCH_MOTION_HOOK = Event::bus()->m_events.input.touch.motion.listen(
        [](const ITouch::SMotionEvent &, Event::SCallbackInfo &) -> void {
            try {
                g_ctx->idleTracker->notifyActivity();
            } catch (const std::exception &e) {
                logErr("touch motion: " + std::string(e.what()));
            } catch (...) {
                logErr("touch motion: unknown exception");
            }
        });

    // Mouse axis (scroll wheel + touchpad scroll) → user activity
    static auto MOUSE_AXIS_HOOK = Event::bus()->m_events.input.mouse.axis.listen(
        [](const IPointer::SAxisEvent &, Event::SCallbackInfo &) -> void {
            try {
                g_ctx->idleTracker->notifyActivity();
            } catch (const std::exception &e) {
                logErr("mouse axis: " + std::string(e.what()));
            } catch (...) {
                logErr("mouse axis: unknown exception");
            }
        });

    // Touchpad swipe gestures → user activity
    static auto SWIPE_BEGIN_HOOK = Event::bus()->m_events.gesture.swipe.begin.listen(
        [](const IPointer::SSwipeBeginEvent &, Event::SCallbackInfo &) -> void {
            try {
                g_ctx->idleTracker->notifyActivity();
            } catch (const std::exception &e) {
                logErr("swipe begin: " + std::string(e.what()));
            } catch (...) {
                logErr("swipe begin: unknown exception");
            }
        });

    static auto SWIPE_END_HOOK = Event::bus()->m_events.gesture.swipe.end.listen(
        [](const IPointer::SSwipeEndEvent &, Event::SCallbackInfo &) -> void {
            try {
                g_ctx->idleTracker->notifyActivity();
            } catch (const std::exception &e) {
                logErr("swipe end: " + std::string(e.what()));
            } catch (...) {
                logErr("swipe end: unknown exception");
            }
        });

    static auto SWIPE_UPDATE_HOOK = Event::bus()->m_events.gesture.swipe.update.listen(
        [](const IPointer::SSwipeUpdateEvent &, Event::SCallbackInfo &) -> void {
            try {
                g_ctx->idleTracker->notifyActivity();
            } catch (const std::exception &e) {
                logErr("swipe update: " + std::string(e.what()));
            } catch (...) {
                logErr("swipe update: unknown exception");
            }
        });

    // Touchpad pinch gestures → user activity
    static auto PINCH_BEGIN_HOOK = Event::bus()->m_events.gesture.pinch.begin.listen(
        [](const IPointer::SPinchBeginEvent &, Event::SCallbackInfo &) -> void {
            try {
                g_ctx->idleTracker->notifyActivity();
            } catch (const std::exception &e) {
                logErr("pinch begin: " + std::string(e.what()));
            } catch (...) {
                logErr("pinch begin: unknown exception");
            }
        });

    static auto PINCH_END_HOOK = Event::bus()->m_events.gesture.pinch.end.listen(
        [](const IPointer::SPinchEndEvent &, Event::SCallbackInfo &) -> void {
            try {
                g_ctx->idleTracker->notifyActivity();
            } catch (const std::exception &e) {
                logErr("pinch end: " + std::string(e.what()));
            } catch (...) {
                logErr("pinch end: unknown exception");
            }
        });

    static auto PINCH_UPDATE_HOOK = Event::bus()->m_events.gesture.pinch.update.listen(
        [](const IPointer::SPinchUpdateEvent &, Event::SCallbackInfo &) -> void {
            try {
                g_ctx->idleTracker->notifyActivity();
            } catch (const std::exception &e) {
                logErr("pinch update: " + std::string(e.what()));
            } catch (...) {
                logErr("pinch update: unknown exception");
            }
        });

    (void)MOUSE_HOOK;
    (void)MOUSE_MOVE_HOOK;
    (void)KEY_HOOK;
    (void)TOUCH_DOWN_HOOK;
    (void)TOUCH_UP_HOOK;
    (void)TOUCH_MOTION_HOOK;
    (void)MOUSE_AXIS_HOOK;
    (void)SWIPE_BEGIN_HOOK;
    (void)SWIPE_END_HOOK;
    (void)SWIPE_UPDATE_HOOK;
    (void)PINCH_BEGIN_HOOK;
    (void)PINCH_END_HOOK;
    (void)PINCH_UPDATE_HOOK;
}

void registerWindowHooks() {
    static auto WINDOW_CLOSE_HOOK = Event::bus()->m_events.window.close.listen([](const PHLWINDOW &w) -> void {
        try {
            // Focus tracking is handled entirely by the window.active hook below.
            // Hyprland fires window.active reliably for every focus transition
            // (window→window, window→desktop, desktop→window).  Preemptively
            // resetting currentFocus here would emit stale Desktop signals when
            // focus actually transfers to another window (e.g. terminal after
            // closing a browser) before window.active catches up.
            (void)w;
        } catch (const std::exception &e) {
            logErr("window close: " + std::string(e.what()));
        } catch (...) {
            logErr("window close: unknown exception");
        }
    });

    static auto WINDOW_FOCUS_HOOK =
        Event::bus()->m_events.window.active.listen([](const PHLWINDOW &w, Desktop::eFocusReason) -> void {
            try {
                if (!w) {
                    // Focus moved to desktop / no window
                    g_ctx->currentFocus.reset();
                    g_ctx->focusedWindow.reset();
                    g_ctx->lockManager->setFocusedApp(std::nullopt);
                } else {
                    // Populate window info from Hyprland's CWindow fields.
                    // Use m_initialClass (stable) instead of m_class (changes at runtime).
                    const auto appIdRaw = w->m_initialClass;
                    const auto title = w->m_title;
                    const auto pid = w->getPID();

                    auto appId = AppId::from_raw(appIdRaw);
                    if (!appId.has_value()) {
                        // Skip focus events for windows without a valid class
                        // (e.g. tooltips, popups). Keep last known focus.
                        return;
                    }

                    const bool shown = g_ctx->lockManager->isOverlayShown(*appId);

                    g_ctx->focusedWindow = w;
                    g_ctx->currentFocus = WindowInfo{
                        .appId = *appId,
                        .title = title,
                        .pid = static_cast<uint32_t>(pid),
                        .uid = g_ctx->uid,
                        .overlayShown = shown,
                    };
                    // LockManager queries g_ctx->currentFocus directly as single
                    // source of truth. setFocusedApp is only for initial sync.
                    g_ctx->lockManager->setFocusedApp(appId);

                    // Re-sync ActiveBlocks on focus change for low-latency
                    // overlay state updates (currently polling-based).
                    if (g_ctx->manager) {
                        g_ctx->manager->readActiveBlocksAsync();
                    }
                }
                if (g_ctx->manager) {
                    g_ctx->manager->emitFocusChanged(g_ctx->currentFocus);
                }
            } catch (const std::exception &e) {
                logErr("window focus: " + std::string(e.what()));
            } catch (...) {
                logErr("window focus: unknown exception");
            }
        });

    (void)WINDOW_CLOSE_HOOK;
    (void)WINDOW_FOCUS_HOOK;
}

/// Register all compositor event listeners. Called ONCE from PLUGIN_INIT.
void registerHooks() {
    registerRenderHook();
    registerInputHooks();
    registerWindowHooks();
}

} // anonymous namespace

// =============================================================================
// Required Hyprland plugin entry points
// =============================================================================

extern "C" APICALL EXPORT std::string PLUGIN_API_VERSION() { return HYPRLAND_API_VERSION; }

extern "C" APICALL EXPORT PLUGIN_DESCRIPTION_INFO PLUGIN_INIT(HANDLE handle) {
    PHANDLE = handle;

    // ── Version hash check (prevents crashes from mismatched headers) ──
    {
        const std::string HASH = __hyprland_api_get_hash();
        const std::string CLIENT_HASH = __hyprland_api_get_client_hash();

        if (HASH != CLIENT_HASH) {
            HyprlandAPI::addNotification(PHANDLE,
                                         "[wellbeing-lockdown] Failure in initialization: Version mismatch (headers "
                                         "ver is not equal to running hyprland ver)",
                                         CHyprColor{1.0, 0.2F, 0.2F, 1.0}, 5000);
            logErr("version mismatch: headers hash '" + CLIENT_HASH + "' != compositor hash '" + HASH + "'");
            throw std::runtime_error("version mismatch: headers hash '" + CLIENT_HASH + "' != compositor hash '" +
                                     HASH + "'");
        }
    }

    // ── Create PluginState (RAII) ──────────────────────────────────────
    auto state = std::make_unique<wellbeing::PluginState>();

    // ── Cache uid in PluginState ───────────────────────────────────────
    state->uid = static_cast<uint32_t>(getuid());

    // ── Create shared LockManager (used by both hooks and WellbeingManager) ──
    state->lockManager = std::make_shared<LockManager>();

    // ── Create BOTH D-Bus connections ───────────────────────────────────
    // Always connect to system and session busses. The daemon may be on
    // either one; resolveActiveDaemonBus() selects the active bus at
    // construction and re-selects on NameOwnerChanged. No background retry
    // thread needed — NameOwnerChanged watches on both busses detect
    // daemon (re)appearance without polling.
    try {
        auto sysConn = sdbus::createSystemBusConnection();
        auto sessConn = sdbus::createSessionBusConnection();
        state->sysConnection = std::shared_ptr<sdbus::IConnection>(sysConn.release());
        state->sessConnection = std::shared_ptr<sdbus::IConnection>(sessConn.release());

        if (!state->sysConnection || !state->sessConnection) {
            logErr("PLUGIN_INIT: failed to create D-Bus connections");
            return PLUGIN_DESCRIPTION_INFO{"", "", "", ""};
        }

        state->manager =
            std::make_unique<WellbeingManager>(state->lockManager, state->sysConnection, state->sessConnection);
    } catch (const std::exception &e) {
        logErr("PLUGIN_INIT: D-Bus init failed: " + std::string(e.what()));
        return PLUGIN_DESCRIPTION_INFO{"", "", "", ""};
    }

    // ── Create IdleTracker (after WellbeingManager is ready) ──────────────
    // The transition callback reads g_ctx at call time (always valid because
    // hooks fire only after g_ctx is installed below, and the callback is
    // destroyed before PluginState during PLUGIN_EXIT).
    {
        auto onTransition = [](IdleState newState) -> void {
            if (!g_ctx || !g_ctx->manager) {
                return;
            }
            switch (newState) {
            case IdleState::Idle:
                g_ctx->manager->emitActivityChanged(FocusActivityTag::Idle);
                logInfo("activity: idle");
                break;
            case IdleState::Active:
                g_ctx->manager->emitActivityChanged(FocusActivityTag::Resumed);
                logInfo("activity: resumed");
                break;
            }
        };
        state->idleTracker =
            std::make_unique<IdleTracker>(std::move(onTransition),
                                          focusedWindowHasIdleInhibitor,    // inhibitCheck — Wayland idle-inhibit
                                          std::chrono::milliseconds(30'000) // 30s threshold
            );
    }

    // ── Install global state after everything is ready ─────────────────
    g_ctx = std::move(state);

    // ── Register compositor event listeners (after bus is ready) ──────
    registerHooks();

    // ── Return description ────────────────────────────────────────────
    return PLUGIN_DESCRIPTION_INFO{
        "wellbeing-lockdown",
        "Digital Wellbeing — compositor plugin for screen-time "
        "management. Tracks focused windows and user activity for "
        "usage-based policies.",
        "Digital Wellbeing Authors",
        "0.2.0",
    };
}

extern "C" APICALL EXPORT void PLUGIN_EXIT() {
    g_ctx.reset();
    PHANDLE = nullptr;
}
