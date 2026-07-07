#include "lockdown.hpp"
#include <algorithm>

using wellbeing::ActionType;
using wellbeing::AppId;

// =============================================================================
// Overlay lifecycle
// =============================================================================

void LockManager::showOverlay(const AppId &appId, uint64_t policyId, BlockReason reason, uint64_t blockedSince,
                              const std::vector<ActionType> &actions, const std::vector<uint8_t> &signature) {
    ActiveOverlay overlay;
    overlay.appId = appId;
    overlay.policyId = policyId;
    overlay.blockedSince = blockedSince;
    overlay.signature = signature;
    overlay.actions = actions;
    overlay.reason = reason;

    // TODO: capture target window geometry from compositor memory for each
    //   window owned by this app and populate windowHandles.
    //   PHLWINDOW target = g_pCompositor->m_pLastFocus.lock();
    //   if (target) {
    //       overlay.windowHandles.push_back(reinterpret_cast<uint64_t>(target.get()));
    //   }
    // Per-window trapping currently relies on m_focusedApp gating instead.

    // Build button rects — preallocate to avoid reallocation in hot path.
    // Two buttons: Extra (if available) then Close.
    overlay.buttons.reserve(actions.size());

    // Centered layout: buttons side-by-side at the lower third of the window.
    // Placeholder coords — replace with window-relative positioning once
    // window geometry capture is implemented.
    const int btnW = 140;
    const int btnH = 40;
    const int btnY = 350; // placeholder Y
    const int stepX = 160;

    for (size_t i = 0; i < actions.size(); ++i) {
        // TODO: position relative to target window:
        //   const int btnX = winX + (winW / 2)
        //       - static_cast<int>(actions.size() * stepX / 2) + i * stepX;
        const int btnX = 200 + static_cast<int>(i * stepX);
        overlay.buttons.push_back(ButtonRect{.x = btnX, .y = btnY, .w = btnW, .h = btnH, .actionId = actions[i]});
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
// Focus gate
// =============================================================================

void LockManager::setFocusedApp(const AppId &appId) { m_focusedApp = appId; }

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
            //
            //   const auto pos = target->m_vRealPosition.value();
            //   const auto size = target->m_vRealSize.value();
            //
            //   // 1. 75 % opaque black backdrop over the entire blocked window.
            //   g_pHyprOpenGL->renderRect(
            //       CBox{pos.x, pos.y, size.x, size.y},
            //       CColor{0.0, 0.0, 0.0, 0.75}
            //   );
            //
            //   // 2. Centered prompt text.
            //   // g_pHyprOpenGL->renderText(...) or a drawText helper.
            //
            //   // 3. Action buttons (filled rects + labels).
            //   for (const auto& btn : overlay.buttons) {
            //       // Button background
            //       g_pHyprOpenGL->renderRect(
            //           CBox{btn.x, btn.y, btn.w, btn.h},
            //           CColor{0.2, 0.5, 0.8, 0.9}  // blue accent
            //       );
            //       // Label text centered on the button rect.
            //   }
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
    if (m_focusedApp.empty() || !m_overlays.contains(m_focusedApp)) {
        return false;
    }

    // Hit-test action buttons for the focused app's overlay in order.
    const auto &buttons = m_overlays.at(m_focusedApp).buttons;
    const bool buttonConsumed = std::ranges::any_of(buttons, [this, x, y](const auto &btn) -> auto {
        if (withinRect(btn, x, y)) {
            if (m_userActionCb) {
                m_userActionCb(m_focusedApp, btn.actionId);
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
    return !m_focusedApp.empty() && m_overlays.contains(m_focusedApp);
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
// Token accessors — per-app
// =============================================================================

auto LockManager::activePolicyId(const AppId &appId) const -> uint64_t { return m_overlays.at(appId).policyId; }

auto LockManager::blockedSince(const AppId &appId) const -> uint64_t { return m_overlays.at(appId).blockedSince; }

auto LockManager::activeSignature(const AppId &appId) const -> const std::vector<uint8_t> & {
    return m_overlays.at(appId).signature;
}

// =============================================================================
// Helpers
// =============================================================================

auto LockManager::withinRect(const ButtonRect &r, double x, double y) -> bool {
    return x >= static_cast<double>(r.x) && x < static_cast<double>(r.x + r.w) && y >= static_cast<double>(r.y) &&
           y < static_cast<double>(r.y + r.h);
}
