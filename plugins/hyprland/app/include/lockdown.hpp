#pragma once

#include <cstdint>
#include <functional>
#include <optional>
#include <string>
#include <unordered_map>
#include <vector>

#include "types.hpp"

// Bring newtypes into scope for LockManager domain API.
using wellbeing::ActionType;
using wellbeing::AppId;
using wellbeing::BlockReason;

// ── Hash for AppId (required by std::unordered_map) ──────────────────────────
namespace std {
template<>
struct hash<wellbeing::AppId> {
    auto operator()(const wellbeing::AppId &appId) const -> size_t { return hash<std::string>{}(appId.value()); }
};
} // namespace std

// ── ButtonRect ───────────────────────────────────────────────────────────────
// Screen-space bounding box for an overlay action button. Used by
// LockManager::onMouseClick() for hit-testing.
struct ButtonRect {
    int x = 0, y = 0, w = 0, h = 0;
    ActionType actionId = ActionType::Extra;
};

// ── ActiveOverlay ─────────────────────────────────────────────────────────────
// Per-app blocking state stored in LockManager::m_overlays.
// All fields come from the daemon-issued show command and are echoed back
// verbatim in UserAction signals. Multiple distinct apps can be blocked
// simultaneously, each with its own ActiveOverlay.
struct ActiveOverlay {
    AppId appId;
    uint64_t policyId = 0;
    uint64_t blockedSince = 0;
    std::vector<ActionType> actions;
    BlockReason reason = BlockReason::AppTimeLimit;
    std::vector<ButtonRect> buttons;
    std::vector<uint64_t> windowHandles; // all windows owned by this app, captured at showOverlay time
};

// ── Error ────────────────────────────────────────────────────────────────────
// Domain error codes for LockManager operations.
enum class LockManagerError : std::uint8_t {
    None,
    AppIdMismatch,   ///< hideOverlay called for wrong app
    NoActiveOverlay, ///< operation requires an active overlay
};

// ── WindowInfo ────────────────────────────────────────────────────────────────
// Describes a focused window. Carried in FocusChanged signal variants and
// returned by the CurrentFocus D-Bus property.
// Uses AppId newtype for type safety.
// See docs/architecture/04-plugin-ipc.md §D-Bus Interface.
struct WindowInfo {
    AppId appId;
    std::string title;
    uint32_t pid = 0;
    uint32_t uid = 0;
    bool overlayShown = false;
};

// ── LockManager ──────────────────────────────────────────────────────────────
// Owns all currently-shown overlay state (multiple per-app overlays).
// Each ActiveOverlay stores the daemon-issued signed token that must be echoed
// back verbatim in UserAction. Input trapping gates on m_focusedApp: only
// the focused window's owning app has its buttons hit-tested and keys swallowed.
//
// All public API uses validated newtypes; raw external data must be converted
// by WellbeingManager (the D-Bus boundary gate) before entering LockManager.
//
// Drawing and input-trapping state lives here. Compositor hooks call
// drawOverlay() / onMouseClick() / onKey() from listeners registered in
// PLUGIN_INIT.
//
// Focus state single source of truth: LockManager queries current focus
// from g_ctx->currentFocus via a getter; it does NOT receive duplicate
// setFocusedApp calls from the focus hook.
class LockManager {
  public:
    LockManager() = default;

    // ── Overlay lifecycle ──────────────────────────────────────────────────

    /// Show overlay for `appId`. All fields come from the daemon's
    /// ActiveBlocks entry. Captures window geometry from compositor for
    /// window-relative button positioning.
    void showOverlay(const AppId &appId, uint64_t policyId, BlockReason reason, uint64_t blockedSince,
                     const std::vector<ActionType> &actions);

    /// Hide overlay for `appId`. Erases the stored ActiveOverlay.
    /// Returns AppIdMismatch if appId is not currently blocked.
    auto hideOverlay(const AppId &appId) -> LockManagerError;

    // ── Focus gate ─────────────────────────────────────────────────────────

    /// Set or clear the currently-focused app. Passing std::nullopt clears
    /// the focused app (e.g. when focus moves to desktop).
    /// LockManager queries g_ctx->currentFocus as the single source of truth;
    /// this setter is used for initial sync and cleanup only.
    void setFocusedApp(std::optional<AppId> appId);

    /// Get the currently-focused app, if any.
    [[nodiscard]] auto getFocusedApp() const -> const std::optional<AppId> & { return m_focusedApp; }

    // ── Compositor hooks (called from Event::bus() listeners) ──────────────

    /// Post-render: draw dark backdrop + prompt + action buttons over all
    /// blocked windows. Called from the RENDER_POST_WINDOW stage listener.
    /// Uses g_pHyprOpenGL (Hyprland internal renderer).
    void drawOverlay();

    /// Mouse click handler. Hit-tests saved button rects for the focused app's
    /// overlay; invokes m_userActionCb on a match. Returns true to swallow.
    auto onMouseClick(double x, double y) -> bool;

    /// Keyboard handler. Returns true when the focused app is blocked so the
    /// compositor swallows all keys.
    auto onKey() -> bool;

    // ── Queries ────────────────────────────────────────────────────────────

    /// True when `windowHandle` belongs to any blocked app.
    [[nodiscard]] auto isTarget(uint64_t windowHandle) const -> bool;

    /// True when the given app_id currently has an active overlay.
    [[nodiscard]] auto isOverlayShown(const AppId &appId) const -> bool { return m_overlays.contains(appId); }

    // ── Callback wiring ────────────────────────────────────────────────────

    using UserActionCb = std::function<void(const AppId &appId, ActionType action)>;

    /// Set the callback invoked when onMouseClick detects a button press.
    /// WellbeingManager sets this in its constructor to call emitUserAction.
    void setUserActionCallback(UserActionCb cb) { m_userActionCb = std::move(cb); }

  private:
    // ── Per-app overlay storage ────────────────────────────────────────────
    std::unordered_map<AppId, ActiveOverlay> m_overlays;

    /// Optional AppId of the currently-focused window. Used to gate
    /// keyboard/mouse input to the focused window's app only.
    /// std::nullopt means desktop / no window focused.
    std::optional<AppId> m_focusedApp;

    UserActionCb m_userActionCb;

    // ── Helpers ────────────────────────────────────────────────────────────
    static auto withinRect(const ButtonRect &r, double x, double y) -> bool;
};
