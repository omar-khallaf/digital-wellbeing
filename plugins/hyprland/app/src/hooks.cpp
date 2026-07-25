// =============================================================================
// Hook registration — extracted from main.cpp
//
// Event::bus() listeners for render, input, and window lifecycle. Registered
// once during PLUGIN_INIT via wellbeing::registerHooks().
// =============================================================================

#include "hooks.hpp"

#include <optional>
#include <string>

#include <hyprland/Compositor.hpp>
#include <hyprland/desktop/view/Window.hpp>
#include <hyprland/event/EventBus.hpp>
#include <hyprland/managers/input/InputManager.hpp>
#include <hyprland/render/OpenGL.hpp>

#include "lockdown.hpp"
#include "logging.hpp"
#include "plugin_state.hpp"
#include "types.hpp"

using wellbeing::AppId;
using wellbeing::g_ctx;
using wellbeing::logErr;
using wellbeing::logInfo;

namespace {

// ── Render hook ──────────────────────────────────────────────────
// Draws overlay after each window and ticks the idle tracker after
// the full frame is complete.

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

// ── Input hooks ──────────────────────────────────────────────────
// All pointer, keyboard, touch, and gesture events notify the idle
// tracker. Pointer clicks and key presses also feed into LockManager
// for overlay input trapping.

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

// ── Window hooks ─────────────────────────────────────────────────
// Tracks focus transitions, window title changes, and initial focus
// recovery on plugin load.

void registerWindowHooks() {
    // Focus tracking handled by window.active hook — fires reliably
    // for every focus transition and avoids stale Desktop signals.
    static auto WINDOW_CLOSE_HOOK =
        Event::bus()->m_events.window.close.listen([](const PHLWINDOW &w) -> void { (void)w; });

    static auto WINDOW_FOCUS_HOOK =
        Event::bus()->m_events.window.active.listen([](const PHLWINDOW &w, Desktop::eFocusReason) -> void {
            try {
                if (!w) {
                    g_ctx->focusState.reset();
                    g_ctx->focusedHyprWindow.reset();
                    g_ctx->lockManager->setFocusedApp(std::nullopt);
                } else {
                    const auto appIdRaw = w->m_initialClass;
                    const auto title = w->m_title;
                    const auto pid = w->getPID();

                    auto appId = AppId::from_raw(appIdRaw);
                    if (!appId.has_value()) {
                        return;
                    }

                    g_ctx->focusedHyprWindow = w;
                    g_ctx->focusState = WindowInfo{
                        .appId = *appId,
                        .title = title,
                        .pid = static_cast<uint32_t>(pid),
                        .uid = g_ctx->uid,
                    };
                    // LockManager queries g_ctx->focusState directly as single
                    // source of truth. setFocusedApp is only for initial sync.
                    g_ctx->lockManager->setFocusedApp(appId);
                    // Overlay state updates are handled reactively via the
                    // BlockedAppsChanged signal subscription in WellbeingManager.
                }
                if (g_ctx->wellbeingManager) {
                    g_ctx->wellbeingManager->emitFocusChanged(g_ctx->focusState);
                }
            } catch (const std::exception &e) {
                logErr("window focus: " + std::string(e.what()));
            } catch (...) {
                logErr("window focus: unknown exception");
            }
        });
    static auto WINDOW_TITLE_HOOK = Event::bus()->m_events.window.title.listen([](const PHLWINDOW &w) -> void {
        try {
            const auto focused = g_ctx->focusedHyprWindow.lock();

            // Startup sync: window.active fired before our hooks were
            // registered, so we never saw the initial focus.  The first
            // title event after plugin load is reliably from the focused
            // window — use it to initialize focus state.
            if (!focused && !g_ctx->focusState.has_value()) {
                const auto appIdRaw = w->m_initialClass;
                const auto title = w->m_title;
                const auto pid = w->getPID();

                auto appId = AppId::from_raw(appIdRaw);
                if (!appId.has_value()) return;

                g_ctx->focusedHyprWindow = w;
                g_ctx->focusState = WindowInfo{
                    .appId = *appId,
                    .title = title,
                    .pid = static_cast<uint32_t>(pid),
                    .uid = g_ctx->uid,
                };
                g_ctx->lockManager->setFocusedApp(appId);
                if (g_ctx->wellbeingManager) g_ctx->wellbeingManager->emitFocusChanged(g_ctx->focusState);
                return;
            }

            if (!focused || focused != w || !g_ctx->focusState.has_value()) return;

            g_ctx->focusState->title = w->m_title;
            if (g_ctx->wellbeingManager) g_ctx->wellbeingManager->emitFocusChanged(g_ctx->focusState);
        } catch (const std::exception &e) {
            logErr("window title: " + std::string(e.what()));
        } catch (...) {
            logErr("window title: unknown exception");
        }
    });

    (void)WINDOW_CLOSE_HOOK;
    (void)WINDOW_FOCUS_HOOK;
    (void)WINDOW_TITLE_HOOK;
}

} // namespace

namespace wellbeing {

// ── Public API ────────────────────────────────────────────────────

void registerHooks() {
    registerRenderHook();
    registerInputHooks();
    registerWindowHooks();
}

auto focusedWindowHasIdleInhibitor() -> bool {
    if (!g_pInputManager) {
        return false;
    }
    const auto window = g_ctx->focusedHyprWindow.lock();
    if (!window) {
        return false;
    }
    return g_pInputManager->isWindowInhibiting(window, false);
}

} // namespace wellbeing
