#pragma once

// =============================================================================
// DbusThread — dedicated D-Bus I/O thread using sd-bus + sd-event.
//
// Owns persistent sd-bus connections to BOTH system and session busses.
// The daemon may live on either bus; m_daemonBus tracks which one.
// Bus selection is performed asynchronously via the event loop
// (NameOwnerChanged events and async method calls) so no synchronous
// D-Bus call ever blocks this thread.
//
// All sd-* resources are RAII-managed via unique_ptr with custom deleters.
// =============================================================================

#include <cstdint>
#include <memory>
#include <optional>
#include <string>
#include <thread>

#include <systemd/sd-bus.h>
#include <systemd/sd-event.h>

#include "thread_channels.hpp"

namespace wellbeing {

// ── RAII deleters for systemd types ─────────────────────────────────────────

struct SdBusDeleter {
    void operator()(sd_bus *b) noexcept {
        if (b != nullptr) {
            sd_bus_unref(b);
        }
    }
};
struct SdEventDeleter {
    void operator()(sd_event *e) noexcept {
        if (e != nullptr) {
            sd_event_unref(e);
        }
    }
};
struct SdEventSourceDeleter {
    void operator()(sd_event_source *s) noexcept {
        if (s != nullptr) {
            sd_event_source_unref(s);
        }
    }
};

using UniqueBus = std::unique_ptr<sd_bus, SdBusDeleter>;
using UniqueEv = std::unique_ptr<sd_event, SdEventDeleter>;
using UniqueSrc = std::unique_ptr<sd_event_source, SdEventSourceDeleter>;

// ── Focus state ────────────────────────────────────────────────────────────

/// Focus state as known by the D-Bus thread (from compositor messages).
struct DbusFocusState {
    std::string wclass;
    std::string wTitle;
    bool blocked = false;
};

/// Per-instance context for NameOwnerChanged matches so the handler
/// knows which physical bus received the signal.
struct NameOwnerCtx {
    class DbusThread *self;
    sd_bus *bus;
};

// ── DbusThread ─────────────────────────────────────────────────────────────

class DbusThread {
  public:
    DbusThread(const DbusThread &) = delete;
    DbusThread(DbusThread &&) = delete;
    auto operator=(const DbusThread &) -> DbusThread & = delete;
    auto operator=(DbusThread &&) -> DbusThread & = delete;
    explicit DbusThread(ThreadChannels &channels);
    ~DbusThread();

    void requestShutdown();
    void join();

  private:
    /// Async bus-selection step (event-loop driven, no sync D-Bus calls).
    enum class BusSelectStep : uint8_t {
        Idle,
        CheckSys,     ///< NameHasOwner on system bus
        CheckSess,    ///< NameHasOwner on session bus
        ActivateSys,  ///< StartServiceByName on system bus
        ReCheckSys,   ///< NameHasOwner on system bus after activation
        ActivateSess, ///< StartServiceByName on session bus
        ReCheckSess,  ///< NameHasOwner on session bus after activation
    };

    // ── Thread entry point ──
    void run();

    // ── Static C callbacks ──
    static auto onCompositorEvent(sd_event_source *src, int fd, uint32_t revents, void *userdata) -> int;
    static auto onRegisterReply(sd_bus_message *msg, void *userdata, sd_bus_error *ret_error) -> int;
    static auto onBlockedAppsReply(sd_bus_message *msg, void *userdata, sd_bus_error *ret_error) -> int;
    static auto onBlockedAppsChanged(sd_bus_message *msg, void *userdata, sd_bus_error *ret_error) -> int;
    static auto onNameOwnerChanged(sd_bus_message *msg, void *userdata, sd_bus_error *ret_error) -> int;
    static auto onLogindSignal(sd_bus_message *msg, void *userdata, sd_bus_error *ret_error) -> int;
    static auto onScreenSaverSignal(sd_bus_message *msg, void *userdata, sd_bus_error *ret_error) -> int;
    static auto onGetFocusState(sd_bus_message *msg, void *userdata, sd_bus_error *ret_error) -> int;
    /// Generic handler for async bus-selection replies (NameHasOwner / StartServiceByName).
    static auto onBusSelectReply(sd_bus_message *msg, void *userdata, sd_bus_error *ret_error) -> int;

    // ── Internal handlers ──
    void drainCompositorMessages();
    void handleBlockedAppsReply(sd_bus_message *msg);
    void handleBlockedAppsChanged(sd_bus_message *msg);
    void handleNameOwnerChanged(sd_bus_message *msg, sd_bus *sourceBus);
    void handleLogindSignal(sd_bus_message *msg);
    void handleScreenSaverSignal(sd_bus_message *msg);

    // ── Async bus selection ──
    void issueNameHasOwner(sd_bus *bus);
    void issueStartServiceByName(sd_bus *bus);
    void advanceBusSelection(sd_bus_message *msg);
    void finishBusSelection(sd_bus *bus);
    static auto parseNameHasOwner(sd_bus_message *msg) -> bool;
    static auto parseStartResult(sd_bus_message *msg) -> bool;

    // ── D-Bus signal emission ──
    /// Emit a fully-specified Event signal (caller provides all fields).
    void emitRawEvent(uint32_t tag, const std::string &wClass, const std::string &wTitle, uint32_t powerTag);
    /// Emit Focus/Block/Unfocus based on current m_focusedApp state.
    void emitCurrentFocusEvent();
    /// Emit a system-event signal (Power, Locked, etc.) with optional power-tag discriminator.
    void emitSystemEvent(EventTag tag, uint32_t powerTag = 0);
    /// True when registered with daemon and daemon bus is available.
    [[nodiscard]] auto canEmit() const -> bool;

    // ── Subscriptions ──
    /// Watch NameOwnerChanged for DAEMON_INTERFACE on a single bus.
    void subscribeNameOwnerChanged(sd_bus *bus);
    /// BlockedAppsChanged — follows the daemon bus (m_daemonBus).
    void subscribeToBlockedApps();
    /// logind PrepareForSleep / PrepareForShutdown — always on system bus.
    void subscribeToLogind(sd_bus *bus);
    /// GNOME ScreenSaver ActiveChanged — always on session bus.
    void subscribeToScreenSaver(sd_bus *bus);

    // ── Daemon lifecycle ──
    /// Async RegisterPlugin (no sync NameHasOwner — event guarantees it).
    auto setupDaemonProxy() -> bool;

    // ── Members ──
    ThreadChannels &m_channels;

    UniqueBus m_sysBus;            ///< System bus connection (RAII).
    UniqueBus m_sessBus;           ///< Session bus connection (RAII).
    sd_bus *m_daemonBus = nullptr; ///< Non-owning: which bus has the daemon.

    UniqueEv m_event;               ///< Event loop (RAII).
    UniqueSrc m_compositorEventSrc; ///< Compositor eventfd source (RAII).

    /// NameOwnerChanged per-bus contexts (RAII members, no heap allocation).
    NameOwnerCtx m_sysNameOwnerCtx;
    NameOwnerCtx m_sessNameOwnerCtx;

    BusSelectStep m_selectStep = BusSelectStep::Idle;

    std::optional<DbusFocusState> m_focusedApp;
    bool m_registered = false;
    bool m_screenLocked = false;
    bool m_daemonOwner = false;
    bool m_shutdownRequested = false;
    std::thread m_thread;
};

} // namespace wellbeing
