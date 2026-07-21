#include "lockdown.hpp"
#include <algorithm>
#include <cstdint>

// Hyprland compositor API for window enumeration and geometry.
// Guarded: test builds don't have Hyprland headers.
#if __has_include(<hyprland/Compositor.hpp>)
#include <hyprland/Compositor.hpp>
#include <hyprland/desktop/view/Window.hpp>
#endif

using wellbeing::ActionType;
using wellbeing::AppId;

// =============================================================================
// Overlay lifecycle
// =============================================================================

void LockManager::showOverlay(const AppId &appId, uint64_t policyId, BlockReason reason, uint64_t blockedSince,
                              const std::vector<ActionType> &actions) {
    ActiveOverlay overlay;
    overlay.appId = appId;
    overlay.policyId = policyId;
    overlay.blockedSince = blockedSince;
    overlay.actions = actions;
    overlay.reason = reason;

    // ── Build button rects — preallocate to avoid reallocation ──────────
    overlay.buttons.reserve(actions.size());

    // ── Capture window geometry from compositor ─────────────────────────
    // Find all compositor windows whose initial class matches this appId.
    // We use m_initialClass because m_class can change after window start.
    // Window handles are stored as raw pointers for later compositor API use.
#if __has_include(<hyprland/Compositor.hpp>)
    {
        const auto &windows = g_pCompositor->m_windows;
        for (const auto &w : windows) {
            if (w->m_initialClass == appId.value()) {
                const auto handle = reinterpret_cast<uint64_t>(w.get());
                overlay.windowHandles.push_back(handle);
            }
        }
    }
#endif

    // ── Build button rects: window-relative positioning ─────────────────
    // If we captured windows, center buttons at the lower third of the
    // first captured window. Otherwise use fallback coordinates.
    constexpr int btnW = 140;
    constexpr int btnH = 40;
    constexpr int stepX = 160;

#if __has_include(<hyprland/Compositor.hpp>)
    if (!overlay.windowHandles.empty()) {
        // Find the window that matches our first handle.
        const auto &windows = g_pCompositor->m_windows;
        for (const auto &ww : windows) {
            if (reinterpret_cast<uint64_t>(ww.get()) == overlay.windowHandles[0]) {
                const auto box = ww->getWindowMainSurfaceBox();
                const int winX = static_cast<int>(box.x);
                const int winY = static_cast<int>(box.y);
                const int winW = static_cast<int>(box.w);
                const int winH = static_cast<int>(box.h);

                // Buttons positioned at lower third of window, centered.
                const int btnY = winY + (winH * 2 / 3);
                const int totalWidth = static_cast<int>(actions.size() * stepX);
                const int startX = winX + ((winW - totalWidth) / 2);

                for (size_t i = 0; i < actions.size(); ++i) {
                    const int btnX = startX + static_cast<int>(i * stepX);
                    overlay.buttons.push_back(
                        ButtonRect{.x = btnX, .y = btnY, .w = btnW, .h = btnH, .actionId = actions[i]});
                }
                break;
            }
        }
    }
#endif

    // Fallback: hardcoded coords when no window geometry available.
    if (overlay.buttons.empty()) {
        constexpr int btnY = 350;
        for (size_t i = 0; i < actions.size(); ++i) {
            const int btnX = 200 + static_cast<int>(i * stepX);
            overlay.buttons.push_back(ButtonRect{.x = btnX, .y = btnY, .w = btnW, .h = btnH, .actionId = actions[i]});
        }
    }

    m_overlays.insert_or_assign(appId, std::move(overlay));
}

auto LockManager::hideOverlay(const AppId &appId) -> LockManagerError {
    if (!m_overlays.contains(appId)) {
        return LockManagerError::AppIdMismatch;
    }
    m_overlays.erase(appId);
    return LockManagerError::None;
}

// =============================================================================
// Focus gate — single source of truth is g_ctx->currentFocus
// =============================================================================

void LockManager::setFocusedApp(std::optional<AppId> appId) { m_focusedApp = std::move(appId); }

// =============================================================================
// Compositor hooks
// =============================================================================

void LockManager::drawOverlay() {
    if (m_overlays.empty()) {
        return;
    }

    for (auto &[appId, overlay] : m_overlays) {
        (void)appId;
        for (auto windowHandle : overlay.windowHandles) {
            (void)windowHandle;
            // TODO: draw backdrop over each blocked window using
            //   g_pHyprOpenGL->renderRect(...). When windowHandles is
            //   empty, draw a single placeholder backdrop per overlay.
        }

        // Placeholder structure: when windowHandles is empty, draw a single
        // backdrop using the first button's position (fixed coords for now).
        if (overlay.windowHandles.empty()) {
            // TODO: render placeholder backdrop + buttons for this overlay
            //   once window geometry capture is implemented.
            //   For now the drawing path is structurally visible but empty.
        }
    }
}

auto LockManager::onMouseClick(double x, double y) -> bool {
    if (!m_focusedApp.has_value() || !m_overlays.contains(*m_focusedApp)) {
        return false;
    }

    // Hit-test action buttons for the focused app's overlay in order.
    const auto &buttons = m_overlays.at(*m_focusedApp).buttons;
    const bool buttonConsumed = std::ranges::any_of(buttons, [this, x, y](const auto &btn) -> auto {
        if (withinRect(btn, x, y)) {
            if (m_userActionCb) {
                m_userActionCb(*m_focusedApp, btn.actionId);
            }
            return true;
        }
        return false;
    });

    if (buttonConsumed) {
        return buttonConsumed;
    }

    // TODO: check if click falls inside the blocked window bounds.
    //   If it does → swallow so the blocked app never receives input.
    //   If outside → let pass through normally.
    return false;
}

auto LockManager::onKey() -> bool {
    // Swallow ALL keyboard input when the focused window's app is blocked.
    return m_focusedApp.has_value() && m_overlays.contains(*m_focusedApp);
}

auto LockManager::isTarget(uint64_t windowHandle) const -> bool {
    if (m_overlays.empty()) {
        return false;
    }
    // TODO: check windowHandle against all overlays' windowHandles once
    //   geometry capture is implemented.
    (void)windowHandle;
    return false;
}

// =============================================================================
// Helpers
// =============================================================================

auto LockManager::withinRect(const ButtonRect &r, double x, double y) -> bool {
    return x >= static_cast<double>(r.x) && x < static_cast<double>(r.x + r.w) && y >= static_cast<double>(r.y) &&
           y < static_cast<double>(r.y + r.h);
}
