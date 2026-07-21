// =============================================================================
// wellbeing-lockdown — Hyprland compositor plugin
//
// Implements the org.wellbeing.v1.Manager D-Bus interface on the resolved bus.
// The daemon bus is resolved via resolveDaemonBus() (4-step: system present →
// session present → activate system → activate session).
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

#include <atomic>
#include <memory>
#include <optional>
#include <stdexcept>
#include <string>
#include <thread>

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
using wellbeing::logErr;
using wellbeing::logInfo;

// Handle returned by PLUGIN_INIT; required by Hyprland plugin API.
inline HANDLE PHANDLE = nullptr;

/// Signals the daemon-retry thread to stop (set false in PLUGIN_EXIT).
inline std::atomic<bool> g_daemonRetryRunning{true};

// =============================================================================
// Event::bus() listeners — registered once in PLUGIN_INIT
// =============================================================================

namespace {

// ── Activity tracking ────────────────────────────────────────────
// Simple idle detection: reset a timer on any input event.
// If no events arrive within IDLE_THRESHOLD_MS, emit ActivityChanged(Idle).
// On any event, emit ActivityChanged(Resumed) and reset.
//
// State lives in g_ctx (PluginState) instead of globals.

constexpr auto IDLE_THRESHOLD_MS = std::chrono::milliseconds(30'000); // 30s

void onUserActivity() {
    g_ctx->lastActivity = std::chrono::steady_clock::now();
    if (g_ctx->idle && g_ctx->manager) {
        g_ctx->idle = false;
        g_ctx->manager->emitActivityChanged(FocusActivityTag::Resumed);
        logInfo("activity: resumed");
    }
}

void registerRenderHook() {
    static auto HOOK = Event::bus()->m_events.render.stage.listen([](eRenderStage stage) -> void {
        if (stage == eRenderStage::RENDER_POST_WINDOW) {
            g_ctx->lockManager->drawOverlay();
        }

        // Idle check: run once per render frame (~60fps but cheap compare).
        if (stage == eRenderStage::RENDER_POST) {
            if (!g_ctx->idle && std::chrono::steady_clock::now() - g_ctx->lastActivity > IDLE_THRESHOLD_MS) {
                g_ctx->idle = true;
                if (g_ctx->manager) {
                    g_ctx->manager->emitActivityChanged(FocusActivityTag::Idle);
                    logInfo("activity: idle");
                }
            }
        }
    });
    (void)HOOK;
}

void registerInputHooks() {
    // Mouse button → user activity + overlay input trapping
    // Coordinates come from g_pInputManager because SButtonEvent has no
    // position field (only button + state).
    static auto MOUSE_HOOK = Event::bus()->m_events.input.mouse.button.listen(
        [](IPointer::SButtonEvent e, Event::SCallbackInfo &info) -> void {
            (void)e;
            onUserActivity();
            const auto coords = g_pInputManager->getMouseCoordsInternal();
            if (g_ctx->lockManager->onMouseClick(static_cast<double>(coords.x), static_cast<double>(coords.y))) {
                info.cancelled = true;
            }
        });

    // Mouse motion → user activity only (no trapping)
    static auto MOUSE_MOVE_HOOK = Event::bus()->m_events.input.mouse.move.listen(
        [](const Vector2D &, Event::SCallbackInfo &) -> void { onUserActivity(); });

    // Keyboard key → user activity + overlay input trapping
    static auto KEY_HOOK = Event::bus()->m_events.input.keyboard.key.listen(
        [](IKeyboard::SKeyEvent e, Event::SCallbackInfo &info) -> void {
            (void)e;
            onUserActivity();
            if (g_ctx->lockManager->onKey()) {
                info.cancelled = true;
            }
        });

    // Touch events → user activity
    static auto TOUCH_DOWN_HOOK = Event::bus()->m_events.input.touch.down.listen(
        [](const ITouch::SDownEvent &, Event::SCallbackInfo &) -> void { onUserActivity(); });

    static auto TOUCH_UP_HOOK = Event::bus()->m_events.input.touch.up.listen(
        [](const ITouch::SUpEvent &, Event::SCallbackInfo &) -> void { onUserActivity(); });

    static auto TOUCH_MOTION_HOOK = Event::bus()->m_events.input.touch.motion.listen(
        [](const ITouch::SMotionEvent &, Event::SCallbackInfo &) -> void { onUserActivity(); });

    (void)MOUSE_HOOK;
    (void)MOUSE_MOVE_HOOK;
    (void)KEY_HOOK;
    (void)TOUCH_DOWN_HOOK;
    (void)TOUCH_UP_HOOK;
    (void)TOUCH_MOTION_HOOK;
}

void registerWindowHooks() {
    static auto WINDOW_OPEN_HOOK = Event::bus()->m_events.window.open.listen([](const PHLWINDOW &w) -> void {
        // Capture the initial class at window open time before it can change.
        // m_initialClass is set once by Hyprland and never changes.
        // Store it in the window's class field for later matching.
        (void)w;
    });

    static auto WINDOW_CLOSE_HOOK = Event::bus()->m_events.window.close.listen([](const PHLWINDOW &w) -> void {
        // If the closed window is the focused one, emit FocusChanged with Desktop.
        if (g_ctx->currentFocus.has_value() && w->m_initialClass == g_ctx->currentFocus->appId.value()) {
            // Window PID-based check for exact match (not just class).
            // The class alone could match a different window of the same app.
            const auto pid = w->getPID();
            if (pid >= 0 && static_cast<uint32_t>(pid) == g_ctx->currentFocus->pid) {
                g_ctx->currentFocus.reset();
                g_ctx->lockManager->setFocusedApp(std::nullopt);
                if (g_ctx->manager) {
                    g_ctx->manager->emitFocusChanged(g_ctx->currentFocus);
                    logInfo("window closed: focus moved to desktop");
                }
            }
        }
    });

    static auto WINDOW_FOCUS_HOOK = Event::bus()->m_events.window.active.listen(
        [](const PHLWINDOW &w, [[maybe_unused]] Desktop::eFocusReason reason) -> void {
            if (!w) {
                // Focus moved to desktop / no window
                g_ctx->currentFocus.reset();
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
                // overlay state updates (see subscribeToDaemonSignals notes).
                if (g_ctx->manager) {
                    g_ctx->manager->readActiveBlocks();
                }
            }
            if (g_ctx->manager) {
                g_ctx->manager->emitFocusChanged(g_ctx->currentFocus);
            }
        });

    (void)WINDOW_OPEN_HOOK;
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

    // ── Initialise activity timer ──────────────────────────────────────
    state->lastActivity = std::chrono::steady_clock::now();

    // ── Create shared LockManager (used by both hooks and WellbeingManager) ──
    state->lockManager = std::make_shared<LockManager>();

    // ── Resolve daemon bus and create connection ───────────────────────
    try {
        const auto busVariant = WellbeingManager::resolveDaemonBus();
        if (busVariant.has_value()) {
            // Daemon found — create connection on the same bus.
            auto conn = (*busVariant == WellbeingManager::BusVariant::System) ? sdbus::createSystemBusConnection()
                                                                              : sdbus::createSessionBusConnection();
            state->dbusConnection = std::shared_ptr<sdbus::IConnection>(conn.release());

            if (!state->dbusConnection) {
                logErr("PLUGIN_INIT: failed to create D-Bus connection");
                return PLUGIN_DESCRIPTION_INFO{"", "", "", ""};
            }

            state->manager = std::make_unique<WellbeingManager>(state->lockManager, state->dbusConnection);
        } else {
            // Daemon not reachable yet — install state + hooks, retry in background.
            logInfo("PLUGIN_INIT: daemon not reachable — retrying in background thread");
            g_ctx = std::move(state);
            registerHooks();

            std::thread([raw = g_ctx.get()]() {
                while (g_daemonRetryRunning.load()) {
                    std::this_thread::sleep_for(std::chrono::seconds(5));
                    if (!g_daemonRetryRunning.load()) break;

                    auto bus = WellbeingManager::resolveDaemonBus();
                    if (bus.has_value()) {
                        auto conn = (*bus == WellbeingManager::BusVariant::System)
                                        ? sdbus::createSystemBusConnection()
                                        : sdbus::createSessionBusConnection();
                        raw->dbusConnection = std::shared_ptr<sdbus::IConnection>(conn.release());
                        raw->manager = std::make_unique<WellbeingManager>(raw->lockManager, raw->dbusConnection);
                        logInfo("daemon retry: connected and registered");
                        break;
                    }
                }
            }).detach();

            return PLUGIN_DESCRIPTION_INFO{
                "wellbeing-lockdown",
                "Digital Wellbeing — compositor plugin for screen-time "
                "management. Tracks focused windows and user activity for "
                "usage-based policies.",
                "Digital Wellbeing Authors",
                "0.2.0",
            };
        }
    } catch (const std::exception &e) {
        logErr("PLUGIN_INIT: D-Bus init failed: " + std::string(e.what()));
        return PLUGIN_DESCRIPTION_INFO{"", "", "", ""};
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
    g_daemonRetryRunning.store(false);
    g_ctx.reset();
    PHANDLE = nullptr;
}
