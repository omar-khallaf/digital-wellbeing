#include <memory>
#include <stdexcept>
#include <string>

#include <unistd.h>

// Hyprland plugin API (headers fetched by the superbuild into staging/include)
#include <hyprland/Compositor.hpp>
#include <hyprland/desktop/view/Window.hpp>         // CWindow (m_initialClass, m_class, m_title, getPID)
#include <hyprland/event/EventBus.hpp>              // Event::bus()
#include <hyprland/managers/input/InputManager.hpp> // g_pInputManager, getMouseCoordsInternal()
#include <hyprland/plugins/PluginAPI.hpp>
#include <hyprland/render/OpenGL.hpp>
#include <sdbus-c++/sdbus-c++.h>

#include "hooks.hpp"
#include "lockdown.hpp"
#include "logging.hpp"
#include "plugin_state.hpp"
#include "types.hpp"
#include "wellbeing_manager.hpp"

using wellbeing::g_ctx;
using wellbeing::IdleState;
using wellbeing::IdleTracker;
using wellbeing::logErr;
using wellbeing::logInfo;
using wellbeing::WellbeingManager;

inline HANDLE PHANDLE = nullptr;

extern "C" APICALL EXPORT std::string PLUGIN_API_VERSION() { return HYPRLAND_API_VERSION; }

extern "C" APICALL EXPORT PLUGIN_DESCRIPTION_INFO PLUGIN_INIT(HANDLE handle) {
    PHANDLE = handle;

    {
        const std::string hash = __hyprland_api_get_hash();
        const std::string client_hash = __hyprland_api_get_client_hash();

        if (hash != client_hash) {
            HyprlandAPI::addNotification(PHANDLE,
                                         "[wellbeing-lockdown] Failure in initialization: Version mismatch (headers "
                                         "ver is not equal to running hyprland ver)",
                                         CHyprColor{1.0, 0.2F, 0.2F, 1.0}, 5000);
            logErr("version mismatch: headers hash '" + client_hash + "' != compositor hash '" + hash + "'");
            throw std::runtime_error("version mismatch: headers hash '" + client_hash + "' != compositor hash '" +
                                     hash + "'");
        }
    }

    auto state = std::make_unique<wellbeing::PluginState>();

    state->uid = static_cast<uint32_t>(getuid());

    state->lockManager = std::make_shared<LockManager>();

    try {
        auto sysConn = sdbus::createSystemBusConnection();
        auto sessConn = sdbus::createSessionBusConnection();
        state->sysConnection = std::shared_ptr<sdbus::IConnection>(sysConn.release());
        state->sessConnection = std::shared_ptr<sdbus::IConnection>(sessConn.release());

        if (!state->sysConnection || !state->sessConnection) {
            logErr("PLUGIN_INIT: failed to create D-Bus connections");
            return PLUGIN_DESCRIPTION_INFO{"", "", "", ""};
        }

        state->wellbeingManager =
            std::make_unique<WellbeingManager>(state->lockManager, state->sysConnection, state->sessConnection);
    } catch (const std::exception &e) {
        logErr("PLUGIN_INIT: D-Bus init failed: " + std::string(e.what()));
        return PLUGIN_DESCRIPTION_INFO{"", "", "", ""};
    }

    // The transition callback reads g_ctx at call time (always valid because
    // hooks fire only after g_ctx is installed below, and the callback is
    // destroyed before PluginState during PLUGIN_EXIT).
    {
        auto onTransition = [](IdleState newState) -> void {
            if (!g_ctx || !g_ctx->wellbeingManager) {
                return;
            }
            switch (newState) {
            case IdleState::Idle:
                g_ctx->wellbeingManager->emitSimpleEvent(wellbeing::EventTag::Idle);
                logInfo("activity: idle");
                break;
            case IdleState::Active:
                g_ctx->wellbeingManager->emitSimpleEvent(wellbeing::EventTag::Resume);
                logInfo("activity: resumed");
                break;
            }
        };
        state->idleTracker = std::make_unique<IdleTracker>(
            std::move(onTransition),
            wellbeing::focusedWindowHasIdleInhibitor // inhibitCheck — Wayland idle-inhibit
        );
    }

    g_ctx = std::move(state);

    wellbeing::registerHooks();

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
