// =============================================================================
// wellbeing-lockdown — Hyprland compositor plugin
//
// Implements the org.wellbeing.v1.Manager D-Bus interface on the resolved bus.
// The daemon bus is resolved via resolveDaemonBus() (4-step: system present →
// session present → activate system → activate session).
// Provides:
//   - Overlay(v)      method   (SignedEnvelope → Ed25519 verify → show/hide)
//   - UserAction      signal   (button click, echoes daemon-issued token)
//   - FocusChanged    signal   (Option<WindowInfo> on every focus switch)
//   - ActivityChanged signal   (idle / resumed)
//   - CurrentSession  property (read-only SessionState variant)
//
// See docs/architecture/04-plugin-ipc.md and 05-daemon-auth.md.
// =============================================================================

#include <memory>
#include <optional>
#include <string>

// Hyprland plugin API (headers fetched by the superbuild into staging/include)
#include <hyprland/event/EventBus.hpp> // Event::bus()
#include <hyprland/plugins/PluginAPI.hpp>
#include <hyprland/render/OpenGL.hpp>
#include <sdbus-c++/sdbus-c++.h>

#include "lockdown.hpp"
#include "logging.hpp"
#include "plugin_state.hpp"
#include "wellbeing_manager.hpp"

using wellbeing::AppId;
using wellbeing::g_ctx;
using wellbeing::logErr;
using wellbeing::logInfo;

// Handle returned by PLUGIN_INIT; required by Hyprland plugin API.
inline HANDLE PHANDLE = nullptr;

// =============================================================================
// Event::bus() listeners — registered once in PLUGIN_INIT
// =============================================================================

namespace {

void registerRenderHook() {
    static auto HOOK = Event::bus()->m_events.render.stage.listen([](eRenderStage stage) -> void {
        if (stage == eRenderStage::RENDER_POST_WINDOW) {
            g_ctx->lockManager->drawOverlay();
        }
    });
    (void)HOOK;
}

void registerInputHooks() {
    static auto MOUSE_HOOK = Event::bus()->m_events.input.mouse.button.listen(
        [](IPointer::SButtonEvent e, Event::SCallbackInfo &info) -> void {
            (void)e;
            // Directed gate happens inside onMouseClick: it only traps when the
            // focused app has an active overlay (per-app_id), so no global
            // "anything locked?" query is needed.
            // TODO: read real mouse coords:
            //   const auto c = g_pInputManager->getMouseCoords();
            //   if (g_ctx->lockManager->onMouseClick(c.x, c.y))
            //       info.cancelled = true;
            if (g_ctx->lockManager->onMouseClick(0.0, 0.0)) {
                info.cancelled = true;
            }
        });

    static auto KEY_HOOK = Event::bus()->m_events.input.keyboard.key.listen(
        [](IKeyboard::SKeyEvent e, Event::SCallbackInfo &info) -> void {
            (void)e;
            if (g_ctx->lockManager->onKey()) {
                info.cancelled = true;
            }
        });

    (void)MOUSE_HOOK;
    (void)KEY_HOOK;
}

void registerWindowHooks() {
    static auto WINDOW_OPEN_HOOK = Event::bus()->m_events.window.open.listen([](const PHLWINDOW &w) -> void {
        (void)w; // TODO: emit FocusChanged if it grabs focus.
    });

    static auto WINDOW_CLOSE_HOOK = Event::bus()->m_events.window.close.listen([](const PHLWINDOW &w) -> void {
        (void)w; // TODO: if w is blocked target, hide overlay.
    });

    static auto WINDOW_FOCUS_HOOK = Event::bus()->m_events.window.active.listen(
        [](const PHLWINDOW &w, [[maybe_unused]] Desktop::eFocusReason reason) -> void {
            if (!w) {
                g_ctx->currentFocus.reset();
                g_ctx->lockManager->setFocusedApp(AppId::from_unchecked(""));
            } else {
                // TODO: populate from w->m_szClass, w->m_szTitle, w->m_iPid
                const auto appId = AppId::from_unchecked(""); // placeholder
                const bool shown = g_ctx->lockManager->isOverlayShown(appId);
                g_ctx->currentFocus = WindowInfo{
                    .appId = appId,
                    .title = "",
                    .pid = 0,
                    .uid = 0,
                    .overlayShown = shown,
                };
                g_ctx->lockManager->setFocusedApp(appId);
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

extern "C" APICALL EXPORT const char *PLUGIN_API_VERSION() { return HYPRLAND_API_VERSION; }

extern "C" APICALL EXPORT PLUGIN_DESCRIPTION_INFO PLUGIN_INIT(HANDLE handle) {
    PHANDLE = handle;

    // ── Create PluginState (RAII) ──────────────────────────────────────
    auto state = std::make_unique<wellbeing::PluginState>();

    // ── Create shared LockManager (used by both hooks and WellbeingManager) ──
    state->lockManager = std::make_shared<LockManager>();

    // ── Resolve daemon bus (4-step) and create named connection ────────
    // First resolve which bus (system or session) hosts org.wellbeing.v1.Daemon.
    // Then create a connection against that bus, claiming a unique well-known
    // name that lets the daemon discover us. See 13-deployment-modes.md.
    const auto busVariant = WellbeingManager::resolveDaemonBus();
    if (!busVariant.has_value()) {
        logErr("PLUGIN_INIT: daemon unreachable on system or session bus — "
               "running degraded (no overlay enforcement until daemon appears)");
        // Open a default connection anyway so we can retry on NameOwnerChanged.
        state->dbusConnection = std::shared_ptr<sdbus::IConnection>(
            sdbus::createSystemBusConnection(sdbus::ServiceName{WellbeingManager::wellKnownBusName()}));
    } else {
        try {
            auto conn =
                (*busVariant == WellbeingManager::BusVariant::System)
                    ? sdbus::createSystemBusConnection(sdbus::ServiceName{WellbeingManager::wellKnownBusName()})
                    : sdbus::createSessionBusConnection(sdbus::ServiceName{WellbeingManager::wellKnownBusName()});
            state->dbusConnection = std::shared_ptr<sdbus::IConnection>(conn.release());
        } catch (const sdbus::Error &) {
            logErr("PLUGIN_INIT: failed to claim well-known name '" + WellbeingManager::wellKnownBusName() +
                   "' on resolved bus");
            return PLUGIN_DESCRIPTION_INFO{"", "", "", ""};
        }
    }

    if (!state->dbusConnection) {
        logErr("PLUGIN_INIT: failed to create D-Bus connection");
        return PLUGIN_DESCRIPTION_INFO{"", "", "", ""};
    }

    // ── Create WellbeingManager ───────────────────────────────────────
    state->manager = std::make_unique<WellbeingManager>(state->lockManager, state->dbusConnection);

    // ── Install global state after everything is ready ─────────────────
    g_ctx = std::move(state);

    // ── Register compositor event listeners (after bus is ready) ──────
    registerHooks();

    // ── Return description ────────────────────────────────────────────
    return PLUGIN_DESCRIPTION_INFO{
        "wellbeing-lockdown",
        "Digital Wellbeing — overlay compositor plugin for screen-time "
        "management. Blocks distracting apps with an input-trapping overlay.",
        "Digital Wellbeing Authors",
        "0.1.0",
    };
}

extern "C" APICALL EXPORT void PLUGIN_EXIT() {
    // PluginState is a unique_ptr — destructor tears down in order:
    //   1. ~WellbeingManager (D-Bus signal handlers removed)
    //   2. ~sdbus::IConnection (D-Bus disconnect)
    //   3. ~LockManager (state cleared)
    // No raw delete needed — RAII handles cleanup.
    g_ctx.reset();
    PHANDLE = nullptr;
}
