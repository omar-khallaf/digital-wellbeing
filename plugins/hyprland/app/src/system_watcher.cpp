// =============================================================================
// System signal watchers — logind (PrepareForSleep / PrepareForShutdown) and
// GNOME ScreenSaver (ActiveChanged) signal subscriptions.
//
// These watchers forward system power-state and screensaver events to the
// daemon via the unified Event signal, making the compositor plugin the sole
// source of truth for all lifecycle events.
//
// Signal flow:
//   logind (system bus) ─── PrepareForSleep(true) ─────→ PowerEvent::Suspend
//   logind (system bus) ─── PrepareForSleep(false) ────→ emitFocusEvent (if unlocked)
//   logind (system bus) ─── PrepareForShutdown(true) ───→ PowerEvent::Shutdown
//   GNOME ScreenSaver ───── ActiveChanged(true) ───────→ Locked + track state
//   GNOME ScreenSaver ───── ActiveChanged(false) ──────→ emitFocusEvent + track state
//
// On resume from suspend, checks whether the screen is still locked (tracked
// via m_screenLocked). If locked, defers focus emission to the unlock handler.
// If already unlocked, emits focus immediately to restart the interval.
//
// See docs/architecture/02-platform.md and 03-linux-platform.md.
// =============================================================================

#include "wellbeing_manager.hpp"

#include <cstring>
#include <string>

#include <sdbus-c++/sdbus-c++.h>

#include "logging.hpp"
#include "plugin_state.hpp"
#include "types.hpp"

using wellbeing::logErr;
using wellbeing::logInfo;

void wellbeing::WellbeingManager::setupSystemWatchers() {
    // ── logind PrepareForSleep + PrepareForShutdown (system bus) ──────
    {
        const auto matchExpr = std::string("type='signal',"
                                           "interface='org.freedesktop.login1.Manager',"
                                           "path='/org/freedesktop/login1'");

        try {
            m_logindSlot = m_sysConn->addMatch(
                matchExpr,
                [this](sdbus::Message msg) -> void {
                    try {
                        const char *member = msg.getMemberName();
                        bool value = false;
                        msg >> value;

                        if (std::strcmp(member, "PrepareForSleep") == 0) {
                            handlePrepareForSleep(value);
                        } else if (std::strcmp(member, "PrepareForShutdown") == 0) {
                            handlePrepareForShutdown(value);
                        }
                    } catch (const std::exception &e) {
                        logErr("logind signal handler: " + std::string(e.what()));
                    } catch (...) {
                        logErr("logind signal handler: unknown exception");
                    }
                },
                sdbus::return_slot);

            logInfo("setupSystemWatchers: subscribed to logind PrepareForSleep + "
                    "PrepareForShutdown on system bus");
        } catch (const sdbus::Error &e) {
            logErr("setupSystemWatchers: logind addMatch failed: " + std::string(e.what()));
        }
    }

    // ── GNOME ScreenSaver ActiveChanged (session bus) ──────────────
    {
        const auto matchExpr = std::string("type='signal',"
                                           "interface='org.gnome.ScreenSaver',"
                                           "member='ActiveChanged',"
                                           "path='/org/gnome/ScreenSaver'");

        try {
            m_screenSaverSlot = m_sessConn->addMatch(
                matchExpr,
                [this](sdbus::Message msg) -> void {
                    try {
                        bool active = false;
                        msg >> active;
                        handleScreenSaverActive(active);
                    } catch (const std::exception &e) {
                        logErr("screensaver signal handler: " + std::string(e.what()));
                    } catch (...) {
                        logErr("screensaver signal handler: unknown exception");
                    }
                },
                sdbus::return_slot);

            logInfo("setupSystemWatchers: subscribed to GNOME ScreenSaver ActiveChanged "
                    "on session bus");
        } catch (const sdbus::Error &e) {
            logInfo("setupSystemWatchers: GNOME ScreenSaver not available (" + std::string(e.what()) +
                    ") — screensaver lock detection disabled");
        }
    }
}

void wellbeing::WellbeingManager::handlePrepareForSleep(bool sleeping) {
    if (!sleeping) {
        // Wake from suspend — re-send current focus only if the screen is already
        // unlocked. If still locked, skip and let the unlock handler emit focus.
        if (m_screenLocked) {
            logInfo("system: resumed from suspend but screen still locked — "
                    "waiting for unlock");
        } else {
            logInfo("system: resumed from suspend — re-emitting current focus");
            if (g_ctx) {
                emitFocusEvent(focusStateSnapshot());
            }
        }
        return;
    }
    logInfo("system: preparing to suspend — emitting PowerEvent::Suspend");
    emitEvent(EventTag::Power, std::string{}, std::string{}, 0, static_cast<uint32_t>(PowerTag::Suspend));
}

void wellbeing::WellbeingManager::handlePrepareForShutdown(bool shuttingDown) {
    if (!shuttingDown) {
        // Shutdown aborted — no event needed.
        return;
    }
    logInfo("system: preparing to shutdown — emitting PowerEvent::Shutdown");
    emitEvent(EventTag::Power, std::string{}, std::string{}, 0, static_cast<uint32_t>(PowerTag::Shutdown));
}

void wellbeing::WellbeingManager::handleScreenSaverActive(bool active) {
    m_screenLocked = active;
    if (!active) {
        // Screen unlocked — re-send current focus to restart the tracking interval.
        logInfo("system: screen unlocked — re-emitting current focus");
        if (g_ctx) {
            emitFocusEvent(focusStateSnapshot());
        }
        return;
    }
    logInfo("system: screensaver activated / screen locked — emitting Locked");
    emitSimpleEvent(EventTag::Locked);
}
