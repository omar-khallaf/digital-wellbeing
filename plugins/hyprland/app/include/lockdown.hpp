#pragma once

#include <cstdint>
#include <functional>
#include <memory>
#include <mutex>
#include <optional>
#include <string>
#include <unordered_map>
#include <vector>

#include "types.hpp"

using wellbeing::ActionType;
using wellbeing::AppClass;
using wellbeing::BlockReason;

namespace std {
template<>
struct hash<wellbeing::AppClass> {
    auto operator()(const wellbeing::AppClass &appClass) const -> size_t {
        return hash<std::string>{}(appClass.value());
    }
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
    AppClass appClass;
    uint64_t policyId = 0;
    uint64_t blockedSince = 0;
    std::vector<ActionType> actions;
    BlockReason reason = BlockReason::AppTimeLimit;
    std::vector<ButtonRect> buttons;
    std::vector<uint64_t> windowHandles; // all windows owned by this app, captured at showOverlay time
};

enum class LockManagerError : std::uint8_t {
    None,
    AppClassMismatch,
    NoActiveOverlay,
};

// Returned by the CurrentFocus D-Bus property.
// See docs/architecture/04-plugin-ipc.md §D-Bus Interface.
struct WindowInfo {
    AppClass appClass;
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

    /// All fields come from the daemon's BlockedApps entry. Captures window
    /// geometry for button positioning.
    void showOverlay(const AppClass &appClass, uint64_t policyId, BlockReason reason, uint64_t blockedSince,
                     const std::vector<ActionType> &actions);

    /// Erases the stored ActiveOverlay. Returns AppClassMismatch if appClass is
    /// not currently blocked.
    auto hideOverlay(const AppClass &appClass) -> LockManagerError;

    /// Set or clear the currently-focused app. Passing std::nullopt clears
    /// the focused app (e.g. when focus moves to desktop).
    /// Used for initial sync and cleanup only; LockManager queries
    /// g_ctx->focusState as the single source of truth.
    void setFocusedApp(std::optional<AppClass> appClass);

    [[nodiscard]] auto getFocusedApp() const -> const std::optional<AppClass> & { return m_focusedApp; }

    /// Pre-windows: refresh window-handle and button-position caches.
    /// Called from the RENDER_PRE_WINDOWS stage listener.
    void refreshOverlay();

    /// Per-window: draw a dark backdrop over the given window if it belongs
    /// to a blocked app. Called from the RENDER_POST_WINDOW stage listener
    /// so the backdrop is naturally covered by overlapping windows above.
    void drawBackdropForHandle(uint64_t windowHandle);

    /// Mouse click handler. Hit-tests saved button rects for the focused
    /// app's overlay; invokes m_userActionCb on a match.
    /// Returns true to swallow the event.
    auto onMouseClick(double x, double y) -> bool;

    [[nodiscard]] auto isTarget(uint64_t windowHandle) const -> bool;

    [[nodiscard]] auto isOverlayShown(const AppClass &appClass) const -> bool {
        std::scoped_lock lock(*m_mutex);
        return m_overlays.contains(appClass);
    }

  private:
    /// Mutex protects m_overlays against concurrent access from D-Bus thread
    /// (showOverlay/hideOverlay) and Hyprland compositor thread
    /// (drawOverlay/onMouseClick/onKey).
    /// Unique ptr to keep LockManager movable (std::mutex is not movable).
    mutable std::unique_ptr<std::mutex> m_mutex{std::make_unique<std::mutex>()};
    std::unordered_map<AppClass, ActiveOverlay> m_overlays;

    /// Unlocked erase — caller must hold m_mutex.
    auto eraseOverlay(const AppClass &appClass) -> LockManagerError;

    /// AppClass of the currently-focused window. Gates keyboard/mouse input
    /// to the focused window's app only. std::nullopt = no focus.
    /// Only accessed from Hyprland compositor thread — no sync needed.
    std::optional<AppClass> m_focusedApp;

    static auto withinRect(const ButtonRect &r, double x, double y) -> bool;
};
