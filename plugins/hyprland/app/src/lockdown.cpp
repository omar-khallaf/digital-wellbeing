#include "lockdown.hpp"
#include <algorithm>
#include <cstdint>
#include <cstring>

// Guarded: test builds don't have Hyprland headers.
#if __has_include(<hyprland/Compositor.hpp>)
#include <hyprland/Compositor.hpp>
#include <hyprland/desktop/view/Window.hpp>
#include <hyprland/render/Renderer.hpp>
#endif

using wellbeing::ActionType;
using wellbeing::AppClass;

void LockManager::showOverlay(const AppClass &appClass, uint64_t policyId, BlockReason reason, uint64_t blockedSince,
                              const std::vector<ActionType> &actions) {
    // Store overlay metadata only. Window capture and button positioning
    // are deferred to refreshOverlay() — windowHandles and buttons are
    // lazily computed on the first render frame, which runs on the
    // compositor thread (RENDER_PRE_WINDOWS hook).
    //
    // DO NOT access g_pCompositor here: showOverlay is called from the
    // D-Bus event-loop thread (via fetchBlocks / BlockedAppsChanged),
    // and compositor state is not thread-safe.
    ActiveOverlay overlay;
    overlay.appClass = appClass;
    overlay.policyId = policyId;
    overlay.blockedSince = blockedSince;
    overlay.actions = actions;
    overlay.reason = reason;

    {
        std::scoped_lock lock(*m_mutex);
        m_overlays.insert_or_assign(appClass, std::move(overlay));
    }
}

auto LockManager::hideOverlay(const AppClass &appClass) -> LockManagerError {
    std::scoped_lock lock(*m_mutex);
    return eraseOverlay(appClass);
}

auto LockManager::eraseOverlay(const AppClass &appClass) -> LockManagerError {
    if (!m_overlays.contains(appClass)) {
        return LockManagerError::AppClassMismatch;
    }
    m_overlays.erase(appClass);
    return LockManagerError::None;
}

void LockManager::setFocusedApp(std::optional<AppClass> appClass) { m_focusedApp = std::move(appClass); }

void LockManager::refreshOverlay() {
    std::scoped_lock lock(*m_mutex);

    if (m_overlays.empty()) return;

    for (auto &[appClass, overlay] : m_overlays) {
#if __has_include(<hyprland/Compositor.hpp>)
        // Refresh window handles and button positions every frame.
        // Re-populating every frame handles window resize/recreation transparently.
        overlay.windowHandles.clear();
        overlay.buttons.clear();

        const auto &windows = g_pCompositor->m_windows;
        for (const auto &w : windows) {
            if (strcasecmp(w->m_initialClass.c_str(), appClass.value().c_str()) == 0)
                overlay.windowHandles.push_back(w->m_stableID);
        }

        // Position buttons at the lower third of the first captured window.
        constexpr int btnW = 140;
        constexpr int btnH = 40;
        constexpr int stepX = 160;

        if (!overlay.windowHandles.empty()) {
            for (const auto &ww : windows) {
                if (ww->m_stableID == overlay.windowHandles[0]) {
                    const auto box = ww->getWindowMainSurfaceBox();
                    const int winX = static_cast<int>(box.x);
                    const int winY = static_cast<int>(box.y);
                    const int winW = static_cast<int>(box.w);
                    const int winH = static_cast<int>(box.h);

                    const int btnY = winY + (winH * 2 / 3);
                    const int totalWidth = static_cast<int>(overlay.actions.size() * stepX);
                    const int startX = winX + ((winW - totalWidth) / 2);

                    overlay.buttons.reserve(overlay.actions.size());
                    for (size_t i = 0; i < overlay.actions.size(); ++i) {
                        const int btnX = startX + static_cast<int>(i * stepX);
                        overlay.buttons.push_back(
                            ButtonRect{.x = btnX, .y = btnY, .w = btnW, .h = btnH, .actionId = overlay.actions[i]});
                    }
                    break;
                }
            }
        }

        // Fallback: hardcoded coords when no window geometry available.
        if (overlay.buttons.empty()) {
            overlay.buttons.reserve(overlay.actions.size());
            constexpr int btnY = 350;
            for (size_t i = 0; i < overlay.actions.size(); ++i) {
                const int btnX = 200 + static_cast<int>(i * stepX);
                overlay.buttons.push_back(
                    ButtonRect{.x = btnX, .y = btnY, .w = btnW, .h = btnH, .actionId = overlay.actions[i]});
            }
        }
#else
        (void)appClass;
        (void)overlay;
#endif
    }
}

void LockManager::drawBackdropForHandle(uint64_t windowHandle) {
#if __has_include(<hyprland/Compositor.hpp>)
    std::scoped_lock lock(*m_mutex);

    if (m_overlays.empty()) return;

    // Check if this window belongs to any blocked app.
    bool isBlocked = false;
    for (const auto &[appClass, overlay] : m_overlays) {
        (void)appClass;
        for (auto h : overlay.windowHandles) {
            if (h == windowHandle) {
                isBlocked = true;
                break;
            }
        }
        if (isBlocked) break;
    }
    if (!isBlocked) return;

    // Find the window by handle and add a backdrop pass element.
    const auto PMONITOR = g_pHyprRenderer->m_renderData.pMonitor.lock();
    if (!PMONITOR) return;

    for (const auto &w : g_pCompositor->m_windows) {
        if (w->m_stableID == windowHandle) {
            const auto gbox = w->getWindowMainSurfaceBox();
            CBox mlbox{gbox.x - PMONITOR->m_position.x, gbox.y - PMONITOR->m_position.y, gbox.w, gbox.h};
            auto rectData = CRectPassElement::SRectData{.box = mlbox, .color = CHyprColor{0.0F, 0.0F, 0.0F, 0.6F}};
            g_pHyprRenderer->m_renderPass.add(makeUnique<CRectPassElement>(rectData));
            return;
        }
    }
#else
    (void)windowHandle;
#endif
}

auto LockManager::onMouseClick(double x, double y) -> bool {
    std::scoped_lock lock(*m_mutex);

    if (!m_focusedApp.has_value() || !m_overlays.contains(*m_focusedApp)) {
        return false;
    }

    // Hit-test action buttons for the focused app's overlay in order.
    // Only ActionType::Close exists — handled locally by hiding the
    // overlay immediately. No other action types remain.
    const auto &buttons = m_overlays.at(*m_focusedApp).buttons;
    const bool buttonConsumed = std::ranges::any_of(buttons, [this, x, y](const auto &btn) -> bool {
        if (withinRect(btn, x, y) && btn.actionId == ActionType::Close) {
            eraseOverlay(*m_focusedApp);
            return true;
        }
        return false;
    });

    if (buttonConsumed) {
        return buttonConsumed;
    }

#if __has_include(<hyprland/Compositor.hpp>)
    for (const auto &[appClass, overlay] : m_overlays) {
        (void)appClass;
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

auto LockManager::isTarget(uint64_t windowHandle) const -> bool {
    std::scoped_lock lock(*m_mutex);
    if (m_overlays.empty()) {
        return false;
    }
#if __has_include(<hyprland/Compositor.hpp>)
    for (const auto &[appClass, overlay] : m_overlays) {
        (void)appClass;
        for (auto handle : overlay.windowHandles) {
            if (handle == windowHandle) {
                return true;
            }
        }
    }
#endif
    return false;
}

auto LockManager::withinRect(const ButtonRect &r, double x, double y) -> bool {
    return x >= static_cast<double>(r.x) && x < static_cast<double>(r.x + r.w) && y >= static_cast<double>(r.y) &&
           y < static_cast<double>(r.y + r.h);
}
