#include "idle_tracker.hpp"

namespace wellbeing {

IdleTracker::IdleTracker(TransitionCallback onTransition, InhibitCheck inhibitCheck,
                         std::chrono::milliseconds threshold)
    : m_threshold(threshold), m_lastActivity(std::chrono::steady_clock::now()), m_onTransition(std::move(onTransition)),
      m_inhibitCheck(std::move(inhibitCheck)) {}

void IdleTracker::notifyActivity() {
    m_lastActivity = std::chrono::steady_clock::now();

    if (!m_idle) {
        return; // already active — nothing to do
    }

    // Transition: Idle → Active
    m_idle = false;
    if (m_onTransition) {
        m_onTransition(IdleState::Active);
    }
}

void IdleTracker::tick() {
    if (m_idle) {
        return; // already idle — nothing to do
    }

    const auto now = std::chrono::steady_clock::now();

    // Fast path: still within threshold
    if (now - m_lastActivity <= m_threshold) {
        return;
    }

    // Threshold elapsed — check inhibitor before transitioning to Idle.
    // When the inhibitor is active we treat it as user activity and reset
    // the timer instead of going idle.
    if (m_inhibitCheck && m_inhibitCheck()) {
        m_lastActivity = now;
        return;
    }

    // Transition: Active → Idle
    m_idle = true;
    if (m_onTransition) {
        m_onTransition(IdleState::Idle);
    }
}

} // namespace wellbeing
