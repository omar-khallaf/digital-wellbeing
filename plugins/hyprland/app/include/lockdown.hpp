#pragma once

#include <cstdint>
#include <functional>
#include <optional>
#include <string>
#include <unordered_map>
#include <vector>

#include "types.hpp"

using wellbeing::ActionType;
using wellbeing::AppId;
using wellbeing::BlockReason;

namespace std {
template<>
struct hash<wellbeing::AppId> {
    auto operator()(const wellbeing::AppId &appId) const -> size_t { return hash<std::string>{}(appId.value()); }
};
} // namespace std

// Screen-space bounding box for an overlay action button.
// Used by LockManager::onMouseClick() for hit-testing.
struct ButtonRect {
    int x = 0, y = 0, w = 0, h = 0;
    ActionType actionId = ActionType::Close;
};

// Per-app blocking state. All fields come from the daemon-issued show command
// and are echoed back verbatim in UserAction signals. Multiple distinct apps
// can be blocked simultaneously, each with its own ActiveOverlay.
struct ActiveOverlay {
    AppId appId;
    uint64_t policyId = 0;
    uint64_t blockedSince = 0;
    std::vector<ActionType> actions;
    BlockReason reason = BlockReason::AppTimeLimit;
    std::vector<ButtonRect> buttons;
    std::vector<uint64_t> windowHandles; // all windows owned by this app, captured at showOverlay time
};

enum class LockManagerError : std::uint8_t {
    None,
    AppIdMismatch,
    NoActiveOverlay,
};

// Carried in FocusChanged signal variants and returned by the CurrentFocus
// D-Bus property.
// See docs/architecture/04-plugin-ipc.md §D-Bus Interface.
struct WindowInfo {
    AppId appId;
    std::string title;
    uint32_t pid = 0;
    uint32_t uid = 0;
};

// Manages all currently-shown overlays (one per blocked app).
// Each ActiveOverlay stores the daemon-issued signed token that must be echoed
// back verbatim in UserAction. Input trapping gates on m_focusedApp: only
// the focused window's owning app has its buttons hit-tested and keys swallowed.
//
// All public API uses validated newtypes; raw external data must be converted
// by WellbeingManager (the D-Bus boundary gate) before entering LockManager.
//
// Compositor hooks call drawOverlay() / onMouseClick() / onKey() from
// listeners registered in PLUGIN_INIT.
//
// Focus state single source of truth: LockManager queries current focus
// from g_ctx->focusState via a getter; it does NOT receive duplicate
// setFocusedApp calls from the focus hook.
class LockManager {
  public:
    LockManager() = default;

    /// Show overlay for `appId`. All fields come from the daemon's
    /// BlockedApps entry. Captures window geometry for button positioning.
    void showOverlay(const AppId &appId, uint64_t policyId, BlockReason reason, uint64_t blockedSince,
                     const std::vector<ActionType> &actions);

    /// Hide overlay for `appId`. Erases the stored ActiveOverlay.
    /// Returns AppIdMismatch if appId is not currently blocked.
    auto hideOverlay(const AppId &appId) -> LockManagerError;

    /// Set or clear the currently-focused app. Passing std::nullopt clears
    /// the focused app (e.g. when focus moves to desktop).
    /// Used for initial sync and cleanup only; LockManager queries
    /// g_ctx->focusState as the single source of truth.
    void setFocusedApp(std::optional<AppId> appId);

    /// Get the currently-focused app, if any.
    [[nodiscard]] auto getFocusedApp() const -> const std::optional<AppId> & { return m_focusedApp; }

    /// Post-render: draw dark backdrop + prompt + action buttons over all
    /// blocked windows. Called from the RENDER_POST_WINDOW stage listener.
    /// Uses g_pHyprOpenGL (Hyprland internal renderer).
    void drawOverlay();

    /// Mouse click handler. Hit-tests saved button rects for the focused
    /// app's overlay; invokes m_userActionCb on a match.
    /// Returns true to swallow the event.
    auto onMouseClick(double x, double y) -> bool;

    /// Keyboard handler. Returns true when the focused app is blocked so
    /// the compositor swallows all keys.
    auto onKey() -> bool;

    /// True when `windowHandle` belongs to any blocked app.
    [[nodiscard]] auto isTarget(uint64_t windowHandle) const -> bool;

    /// True when the given app_id currently has an active overlay.
    [[nodiscard]] auto isOverlayShown(const AppId &appId) const -> bool { return m_overlays.contains(appId); }

  private:
    std::unordered_map<AppId, ActiveOverlay> m_overlays;

    /// AppId of the currently-focused window. Gates keyboard/mouse input
    /// to the focused window's app only. std::nullopt = no focus.
    std::optional<AppId> m_focusedApp;

    static auto withinRect(const ButtonRect &r, double x, double y) -> bool;
};
