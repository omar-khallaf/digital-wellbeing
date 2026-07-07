#pragma once

#include <cstdint>
#include <memory>
#include <optional>
#include <string>

#include <sdbus-c++/sdbus-c++.h>

#include "lockdown.hpp"

// WellbeingManager — D-Bus org.wellbeing.v1.Manager interface implementation.
//
// It holds shared_ptr<LockManager> + shared_ptr<sdbus::IConnection> shared with
// PluginState (PluginState is the sole owner and outlives this manager) and
// owns its D-Bus object.
//
// See docs/architecture/04-plugin-ipc.md and 05-daemon-auth.md.
class WellbeingManager {
  public:
    WellbeingManager(std::shared_ptr<LockManager> lockManager, std::shared_ptr<sdbus::IConnection> connection);

    // ── Reverse discovery ──────────────────────────────────────────────
    void registerWithDaemon();

    // ── Signal emission ────────────────────────────────────────────────
    void emitUserAction(const std::string &appId, ActionType action);
    void emitFocusChanged(const std::optional<WindowInfo> &info);
    void emitActivityChanged(bool idle);

    // ── Daemon bus resolution ─────────────────────────────────────────
    /// 4-step resolution: system present → session present → activate system
    /// → activate session. Returns system bus or session bus, std::nullopt
    /// if unreachable. See 13-deployment-modes.md §Plugin Resolution.
    enum class BusVariant { System, Session };
    static auto resolveDaemonBus() -> std::optional<BusVariant>;

    // ── Instance identity ──────────────────────────────────────────────
    static auto instanceId() -> std::string;
    static auto wellKnownBusName() -> std::string;

  private:
    // ── Overlay handler (dispatches to show/hide helpers) ──────────────
    auto handleOverlay(const sdbus::Variant &envelope) -> bool;
    auto tryShowOverlay(sdbus::Variant &payload) -> bool;
    auto tryHideOverlay(sdbus::Variant &payload) -> bool;

    std::shared_ptr<sdbus::IConnection> m_conn;
    std::unique_ptr<sdbus::IObject> m_object;
    std::shared_ptr<LockManager> m_lockManager;
};
