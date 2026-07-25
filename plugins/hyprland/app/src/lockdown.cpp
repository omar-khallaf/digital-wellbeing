#include "lockdown.hpp"
#include <algorithm>
#include <cstdint>

// Hyprland compositor API for window enumeration and geometry.
// Guarded: test builds don't have Hyprland headers.
#if __has_include(<hyprland/Compositor.hpp>)
#include <hyprland/Compositor.hpp>
#include <hyprland/desktop/view/Window.hpp>
#include <hyprland/render/OpenGL.hpp>
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
                overlay.windowHandles.push_back(w->m_stableID);
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
            if (ww->m_stableID == overlay.windowHandles[0]) {
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
// Focus gate — single source of truth is g_ctx->focusState
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
#if __has_include(<hyprland/Compositor.hpp>)
            const auto &windows = g_pCompositor->m_windows;
            for (const auto &w : windows) {
                if (w->m_stableID == windowHandle) {
                    const auto box = w->getWindowMainSurfaceBox();
                    Render::GL::g_pHyprOpenGL->renderRect(box, CHyprColor{0.0F, 0.0F, 0.0F, 0.6F}, {});
                    break;
                }
            }
#endif
        }

        if (overlay.windowHandles.empty()) {
#if __has_include(<hyprland/Compositor.hpp>)
            const CBox fallbackBox = {100, 100, 800, 600};
            Render::GL::g_pHyprOpenGL->renderRect(fallbackBox, CHyprColor{0.0F, 0.0F, 0.0F, 0.6F}, {});
#endif
        }
    }
}

auto LockManager::onMouseClick(double x, double y) -> bool {
    if (!m_focusedApp.has_value() || !m_overlays.contains(*m_focusedApp)) {
        return false;
    }

    // Hit-test action buttons for the focused app's overlay in order.
    // Only ActionType::Close exists — handled locally by hiding the
    // overlay immediately. No other action types remain.
    const auto &buttons = m_overlays.at(*m_focusedApp).buttons;
    const bool buttonConsumed = std::ranges::any_of(buttons, [this, x, y](const auto &btn) -> bool {
        if (withinRect(btn, x, y) && btn.actionId == ActionType::Close) {
            hideOverlay(*m_focusedApp);
            return true;
        }
        return false;
    });

    if (buttonConsumed) {
        return buttonConsumed;
    }

#if __has_include(<hyprland/Compositor.hpp>)
    for (const auto &[appId, overlay] : m_overlays) {
        (void)appId;
        for (auto handle : overlay.windowHandles) {
            for (const auto &w : g_pCompositor->m_windows) {
                if (w->m_stableID == handle) {
                    const auto box = w->getWindowMainSurfaceBox();
                    if (x >= box.x && x < box.x + box.w && y >= box.y && y < box.y + box.h) {
                        return true;
                    }
                    break;
                }
            }
        }
    }
#endif
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
#if __has_include(<hyprland/Compositor.hpp>)
    for (const auto &[appId, overlay] : m_overlays) {
        (void)appId;
        for (auto handle : overlay.windowHandles) {
            if (handle == windowHandle) {
                return true;
            }
        }
    }
#endif
    return false;
}

// =============================================================================
// Helpers
// =============================================================================

auto LockManager::withinRect(const ButtonRect &r, double x, double y) -> bool {
    return x >= static_cast<double>(r.x) && x < static_cast<double>(r.x + r.w) && y >= static_cast<double>(r.y) &&
           y < static_cast<double>(r.y + r.h);
}
