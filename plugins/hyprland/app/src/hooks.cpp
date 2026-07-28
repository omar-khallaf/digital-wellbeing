// =============================================================================
// Hook registration — render, input, and window lifecycle hooks.
//
// All hooks run on the compositor thread (Hyprland event loop). Focus state
// is pushed to the D-Bus thread via lock-free SPSC. No shared
// mutable state between threads — the compositor owns LockManager and
// IdleTracker, the D-Bus thread owns sd-bus.
// =============================================================================

#include "hooks.hpp"

#include <optional>
#include <string>

#include <hyprland/Compositor.hpp>
#include <hyprland/config/shared/actions/ConfigActions.hpp>
#include <hyprland/desktop/view/Window.hpp>
#include <hyprland/event/EventBus.hpp>
#include <hyprland/managers/SeatManager.hpp>
#include <hyprland/managers/input/InputManager.hpp>
#include <hyprland/render/OpenGL.hpp>
#include <hyprland/render/Renderer.hpp>

#include <sys/eventfd.h>

#include "lockdown.hpp"
#include "logging.hpp"
#include "plugin_state.hpp"
#include "types.hpp"

using Config::Actions::closeWindow;
using wellbeing::logErr;
using wellbeing::logInfo;
using wellbeing::WindowClass;

// Focused window tracked from window.active callback.
// Only accessed on the compositor thread — no mutex needed.
static PHLWINDOWREF g_focusedWindow;

namespace {

constexpr int BTN_W = 140;
constexpr int BTN_H = 36;

// ── Render-pass overlay helpers ────────────────────────────────────────────

/// Draw a dark semi-transparent backdrop over the given window bounds,
/// then a "Close Window" button at the centre, both via Hyprland's render
/// pass API (CRectPassElement + CTexPassElement).
void drawBlockedOverlay(const Desktop::View::CWindow &window) {
    const auto PMONITOR = g_pHyprRenderer->m_renderData.pMonitor.lock();
    if (!PMONITOR) {
        return;
    }

    const auto gbox = window.getWindowMainSurfaceBox();

    // Monitor-relative coordinates (render pass operates in monitor-local space).
    const double rx = gbox.x - PMONITOR->m_position.x;
    const double ry = gbox.y - PMONITOR->m_position.y;
    const double rw = gbox.w;
    const double rh = gbox.h;

    // ── Dark backdrop ──
    g_pHyprRenderer->m_renderPass.add(makeUnique<CRectPassElement>(CRectPassElement::SRectData{
        .box = CBox{rx, ry, rw, rh},
        .color = CHyprColor{0.0F, 0.0F, 0.0F, 0.65F},
    }));

    // ── Close button (centred) ──
    const double btnX = rx + ((rw - BTN_W) / 2.0);
    const double btnY = ry + ((rh - BTN_H) / 2.0);

    g_pHyprRenderer->m_renderPass.add(makeUnique<CRectPassElement>(CRectPassElement::SRectData{
        .box = CBox{btnX, btnY, BTN_W, BTN_H},
        .color = CHyprColor{0.85F, 0.15F, 0.15F, 0.9F},
        .round = 6,
    }));

    // ── "Close Window" text ──
    auto textTex = g_pHyprRenderer->renderText("Close Window", CHyprColor{1.0F, 1.0F, 1.0F, 1.0F}, 14);
    if (textTex) {
        const double texW = textTex->m_size.x;
        const double texH = textTex->m_size.y;
        const double textX = btnX + ((BTN_W - texW) / 2.0);
        const double textY = btnY + ((BTN_H - texH) / 2.0);

        g_pHyprRenderer->m_renderPass.add(makeUnique<CTexPassElement>(CTexPassElement::SRenderData{
            .tex = textTex,
            .box = CBox{textX, textY, texW, texH},
        }));
    }
}

// ── Render hook ──────────────────────────────────────────────────

void registerRenderHook() {
    static auto HOOK = Event::bus()->m_events.render.stage.listen([](eRenderStage stage) -> void {
        try {
            if (stage == eRenderStage::RENDER_POST_WINDOW) {
                auto currentWindow = g_pHyprRenderer->m_renderData.currentWindow.lock();
                if (!currentWindow) {
                    return;
                }

                // Render overlay on every visible blocked window, regardless of
                // focus. RENDER_POST_WINDOW only fires for windows being
                // composited (current workspace), so this naturally limits the
                // overlay to what's on screen.
                if (wellbeing::g_ps && wellbeing::g_ps->lockManager &&
                    wellbeing::g_ps->lockManager->isBlocked(currentWindow->m_initialClass)) {
                    drawBlockedOverlay(*currentWindow);
                }
            }

            if (stage == eRenderStage::RENDER_POST) {
                if (wellbeing::g_ps && wellbeing::g_ps->idleTracker) {
                    wellbeing::g_ps->idleTracker->tick();
                }
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

void registerInputHooks() {
    // Mouse button → user activity + overlay input trapping
    static auto MOUSE_HOOK = Event::bus()->m_events.input.mouse.button.listen(
        [](IPointer::SButtonEvent, Event::SCallbackInfo &info) -> void {
            try {
                if (!wellbeing::g_ps) {
                    return;
                }
                wellbeing::g_ps->idleTracker->notifyActivity();

                const auto coords = g_pInputManager->getMouseCoordsInternal();
                const auto x = static_cast<double>(coords.x);
                const auto y = static_cast<double>(coords.y);

                auto focused = g_focusedWindow.lock();
                if (!focused) {
                    return;
                }

                if (!wellbeing::g_ps->lockManager->isBlocked(focused->m_initialClass)) {
                    return;
                }

                const auto box = focused->getWindowMainSurfaceBox();

                // Check close button (centered).
                const auto btnX = static_cast<int>(box.x) + ((box.w - BTN_W) / 2);
                const auto btnY = static_cast<int>(box.y) + ((box.h - BTN_H) / 2);

                if (x >= btnX && x < btnX + BTN_W && y >= btnY && y < btnY + BTN_H) {
                    // Close button hit — close the focused window.
                    closeWindow(focused);
                    info.cancelled = true;
                    return;
                }

                // Check if click is within the focused window bounds.
                if (x >= box.x && x < box.x + box.w && y >= box.y && y < box.y + box.h) {
                    info.cancelled = true; // swallow
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
                if (wellbeing::g_ps) {
                    wellbeing::g_ps->idleTracker->notifyActivity();
                }
            } catch (const std::exception &e) {
                logErr("mouse move: " + std::string(e.what()));
            } catch (...) {
                logErr("mouse move: unknown exception");
            }
        });

    // Keyboard key → user activity + redirect away from blocked window
    static auto KEY_HOOK =
        Event::bus()->m_events.input.keyboard.key.listen([](IKeyboard::SKeyEvent, Event::SCallbackInfo &) -> void {
            try {
                if (!wellbeing::g_ps) {
                    return;
                }
                wellbeing::g_ps->idleTracker->notifyActivity();

                auto focused = g_focusedWindow.lock();
                if (!focused) {
                    return;
                }

                if (wellbeing::g_ps->lockManager->isBlocked(focused->m_initialClass)) {
                    g_pSeatManager->setKeyboardFocus(nullptr);
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
                if (wellbeing::g_ps) {
                    wellbeing::g_ps->idleTracker->notifyActivity();
                }
            } catch (const std::exception &e) {
                logErr("touch down: " + std::string(e.what()));
            } catch (...) {
                logErr("touch down: unknown exception");
            }
        });

    static auto TOUCH_UP_HOOK =
        Event::bus()->m_events.input.touch.up.listen([](const ITouch::SUpEvent &, Event::SCallbackInfo &) -> void {
            try {
                if (wellbeing::g_ps) {
                    wellbeing::g_ps->idleTracker->notifyActivity();
                }
            } catch (const std::exception &e) {
                logErr("touch up: " + std::string(e.what()));
            } catch (...) {
                logErr("touch up: unknown exception");
            }
        });

    static auto TOUCH_MOTION_HOOK = Event::bus()->m_events.input.touch.motion.listen(
        [](const ITouch::SMotionEvent &, Event::SCallbackInfo &) -> void {
            try {
                if (wellbeing::g_ps) {
                    wellbeing::g_ps->idleTracker->notifyActivity();
                }
            } catch (const std::exception &e) {
                logErr("touch motion: " + std::string(e.what()));
            } catch (...) {
                logErr("touch motion: unknown exception");
            }
        });

    // Mouse axis (scroll wheel) → user activity
    static auto MOUSE_AXIS_HOOK = Event::bus()->m_events.input.mouse.axis.listen(
        [](const IPointer::SAxisEvent &, Event::SCallbackInfo &) -> void {
            try {
                if (wellbeing::g_ps) {
                    wellbeing::g_ps->idleTracker->notifyActivity();
                }
            } catch (const std::exception &e) {
                logErr("mouse axis: " + std::string(e.what()));
            } catch (...) {
                logErr("mouse axis: unknown exception");
            }
        });

    // Swipe gestures → user activity
    static auto SWIPE_BEGIN_HOOK = Event::bus()->m_events.gesture.swipe.begin.listen(
        [](const IPointer::SSwipeBeginEvent &, Event::SCallbackInfo &) -> void {
            try {
                if (wellbeing::g_ps) {
                    wellbeing::g_ps->idleTracker->notifyActivity();
                }
            } catch (const std::exception &e) {
                logErr("swipe begin: " + std::string(e.what()));
            } catch (...) {
                logErr("swipe begin: unknown exception");
            }
        });

    static auto SWIPE_END_HOOK = Event::bus()->m_events.gesture.swipe.end.listen(
        [](const IPointer::SSwipeEndEvent &, Event::SCallbackInfo &) -> void {
            try {
                if (wellbeing::g_ps) {
                    wellbeing::g_ps->idleTracker->notifyActivity();
                }
            } catch (const std::exception &e) {
                logErr("swipe end: " + std::string(e.what()));
            } catch (...) {
                logErr("swipe end: unknown exception");
            }
        });

    static auto SWIPE_UPDATE_HOOK = Event::bus()->m_events.gesture.swipe.update.listen(
        [](const IPointer::SSwipeUpdateEvent &, Event::SCallbackInfo &) -> void {
            try {
                if (wellbeing::g_ps) {
                    wellbeing::g_ps->idleTracker->notifyActivity();
                }
            } catch (const std::exception &e) {
                logErr("swipe update: " + std::string(e.what()));
            } catch (...) {
                logErr("swipe update: unknown exception");
            }
        });

    // Pinch gestures → user activity
    static auto PINCH_BEGIN_HOOK = Event::bus()->m_events.gesture.pinch.begin.listen(
        [](const IPointer::SPinchBeginEvent &, Event::SCallbackInfo &) -> void {
            try {
                if (wellbeing::g_ps) {
                    wellbeing::g_ps->idleTracker->notifyActivity();
                }
            } catch (const std::exception &e) {
                logErr("pinch begin: " + std::string(e.what()));
            } catch (...) {
                logErr("pinch begin: unknown exception");
            }
        });

    static auto PINCH_END_HOOK = Event::bus()->m_events.gesture.pinch.end.listen(
        [](const IPointer::SPinchEndEvent &, Event::SCallbackInfo &) -> void {
            try {
                if (wellbeing::g_ps) {
                    wellbeing::g_ps->idleTracker->notifyActivity();
                }
            } catch (const std::exception &e) {
                logErr("pinch end: " + std::string(e.what()));
            } catch (...) {
                logErr("pinch end: unknown exception");
            }
        });

    static auto PINCH_UPDATE_HOOK = Event::bus()->m_events.gesture.pinch.update.listen(
        [](const IPointer::SPinchUpdateEvent &, Event::SCallbackInfo &) -> void {
            try {
                if (wellbeing::g_ps) {
                    wellbeing::g_ps->idleTracker->notifyActivity();
                }
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

/// Push a message to the D-Bus thread via chan C.
template<typename T>
void pushToDbus(T &&msg) {
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

void registerWindowHooks() {
    // Focus tracking via window.active — fires on every focus transition.
    static auto WINDOW_FOCUS_HOOK =
        Event::bus()->m_events.window.active.listen([](const PHLWINDOW &w, Desktop::eFocusReason) -> void {
            try {
                if (!w) {
                    g_focusedWindow.reset();
                    pushToDbus(wellbeing::FocusUpdate{
                        .wclass = std::nullopt,
                        .wTitle = std::string{},
                    });
                    return;
                }

                g_focusedWindow = w;

                // Emit unfocus when window class is empty (scratchpads,
                // hidden windows, special workspaces with no meaningful class).
                if (w->m_initialClass.empty()) {
                    pushToDbus(wellbeing::FocusUpdate{
                        .wclass = std::nullopt,
                        .wTitle = w->m_title,
                    });
                    return;
                }

                const bool blocked = wellbeing::g_ps && wellbeing::g_ps->lockManager &&
                                     wellbeing::g_ps->lockManager->isBlocked(w->m_initialClass);

                if (blocked) {
                    pushToDbus(wellbeing::BlockedFocus{
                        .wclass = w->m_initialClass,
                        .wTitle = w->m_title,
                    });
                } else {
                    pushToDbus(wellbeing::FocusUpdate{
                        .wclass = w->m_initialClass,
                        .wTitle = w->m_title,
                    });
                }
            } catch (const std::exception &e) {
                logErr("window focus: " + std::string(e.what()));
            } catch (...) {
                logErr("window focus: unknown exception");
            }
        });

    // Title change of the focused window → re-push FocusChanged.
    static auto WINDOW_TITLE_HOOK = Event::bus()->m_events.window.title.listen([](const PHLWINDOW &w) -> void {
        try {
            auto focused = g_focusedWindow.lock();
            if (!focused || focused != w) {
                return;
            }

            // Windows with empty class are treated as unfocused; skip title updates.
            if (focused->m_initialClass.empty()) {
                return;
            }

            const bool blocked = wellbeing::g_ps && wellbeing::g_ps->lockManager &&
                                 wellbeing::g_ps->lockManager->isBlocked(focused->m_initialClass);

            if (blocked) {
                pushToDbus(wellbeing::BlockedFocus{
                    .wclass = focused->m_initialClass,
                    .wTitle = focused->m_title,
                });
            } else {
                pushToDbus(wellbeing::FocusUpdate{
                    .wclass = focused->m_initialClass,
                    .wTitle = focused->m_title,
                });
            }
        } catch (const std::exception &e) {
            logErr("window wTitle: " + std::string(e.what()));
        } catch (...) {
            logErr("window wTitle: unknown exception");
        }
    });

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
    auto focused = g_focusedWindow.lock();
    if (!focused) {
        return false;
    }
    return g_pInputManager->isWindowInhibiting(focused, false);
}

auto focusedWindowClass() -> std::string {
    auto focused = g_focusedWindow.lock();
    if (!focused) {
        return {};
    }
    auto wc = WindowClass::from_raw(focused->m_initialClass);
    return wc.value_or(WindowClass{}).value();
}

auto focusedWindowTitle() -> std::string {
    auto focused = g_focusedWindow.lock();
    if (!focused) {
        return {};
    }
    return focused->m_title;
}

} // namespace wellbeing
