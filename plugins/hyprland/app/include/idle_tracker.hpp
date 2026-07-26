#pragma once

#include <chrono>
#include <cstdint>
#include <functional>

namespace wellbeing {

enum class IdleState : uint8_t {
    Active,
    Idle,
};

// Two entry points:
//   - notifyActivity() called from input hooks (mouse, keyboard, touch)
//   - tick()           called once per render frame (RENDER_POST render hook)
// On every state transition (Active→Idle or Idle→Active) the injected
// TransitionCallback fires. This decouples idle detection from D-Bus signal
// emission, logging, or any other side-effect — the callback IS the side-effect.
// An optional InhibitCheck can be injected to suppress the Active→Idle
// transition (e.g. when the focused window asserts the Wayland idle-inhibit
// protocol). When the check returns true, the idle timer resets instead of
// transitioning to Idle.
class IdleTracker {
  public:
    using TransitionCallback = std::function<void(IdleState)>;
    using InhibitCheck = std::function<bool()>;

    explicit IdleTracker(TransitionCallback onTransition, InhibitCheck inhibitCheck = nullptr,
                         std::chrono::milliseconds threshold = std::chrono::milliseconds(IDLE_THRESHOLD_MILLISECS));

    /// Called by input hooks. Resets the idle timer. If currently Idle,
    /// transitions to Active and fires the TransitionCallback.
    void notifyActivity();

    /// Called once per render frame (RENDER_POST). If Active and the
    /// threshold has elapsed (and no inhibitor is active), transitions to
    /// Idle and fires the TransitionCallback.
    void tick();

    [[nodiscard]] auto isIdle() const noexcept -> bool { return m_idle; }
    [[nodiscard]] auto lastActivity() const noexcept -> std::chrono::steady_clock::time_point { return m_lastActivity; }

  private:
    std::chrono::milliseconds m_threshold;
    std::chrono::steady_clock::time_point m_lastActivity;
    bool m_idle = false;
    TransitionCallback m_onTransition;
    InhibitCheck m_inhibitCheck;
    static constexpr auto IDLE_THRESHOLD_MILLISECS = 30'000;
};

} // namespace wellbeing
