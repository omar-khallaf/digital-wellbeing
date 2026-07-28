// =============================================================================
// DbusThread — dedicated D-Bus I/O thread using sd-bus + sd-event.
//
// All D-Bus IPC happens on this single thread. Communicates with the
// compositor thread via lock-free SPSC queues + eventfds.
//
// Bus selection (4-step) is performed asynchronously via the event loop:
// no synchronous D-Bus call ever blocks this thread.
// =============================================================================

#include "dbus_thread.hpp"

#include <cstdint>
#include <cstring>
#include <string>

#include <sys/eventfd.h>
#include <unistd.h>

#include "logging.hpp"

using wellbeing::logErr;
using wellbeing::logInfo;

namespace wellbeing {

// ═════════════════════════════════════════════════════════════════════════════
// Helpers
// ═════════════════════════════════════════════════════════════════════════════

/// Build and send Event signal with variant containing struct(ussu).
static void sendDbusSignal(sd_bus *bus, uint32_t tag, const char *wClass, const char *wTitle, uint32_t powerTag) {
    sd_bus_message *m = nullptr;
    int r = sd_bus_message_new_signal(bus, &m, MANAGER_OBJECT_PATH, MANAGER_INTERFACE, EVENT_SIGNAL);
    if (r < 0) {
        logErr("sendDbusSignal: new_signal failed");
        return;
    }

    r = sd_bus_message_open_container(m, 'v', "(ussu)");
    if (r < 0) {
        sd_bus_message_unref(m);
        return;
    }

    r = sd_bus_message_append(m, "(ussu)", tag, wClass, wTitle, powerTag);
    if (r < 0) {
        sd_bus_message_unref(m);
        return;
    }

    r = sd_bus_message_close_container(m);
    if (r < 0) {
        sd_bus_message_unref(m);
        return;
    }

    r = sd_bus_send(bus, m, nullptr);
    if (r < 0) {
        logErr("sendDbusSignal: sd_bus_send failed");
    }

    sd_bus_message_unref(m);
}

// ═════════════════════════════════════════════════════════════════════════════
// Static member callbacks
// ═════════════════════════════════════════════════════════════════════════════

auto DbusThread::onCompositorEvent(sd_event_source * /*unused*/, int fd, uint32_t /*unused*/, void *userdata) -> int {
    auto *self = static_cast<DbusThread *>(userdata);
    eventfd_t val = 0;
    eventfd_read(fd, &val);
    self->drainCompositorMessages();
    return 0;
}

auto DbusThread::onRegisterReply(sd_bus_message *msg, void *userdata, sd_bus_error * /*unused*/) -> int {
    auto *self = static_cast<DbusThread *>(userdata);
    if (sd_bus_message_is_method_error(msg, nullptr) != 0) {
        logInfo("RegisterPlugin failed — NameOwnerChanged will retry");
        return 0;
    }
    logInfo("Registered with daemon");
    self->m_registered = true;

    sd_bus_call_method_async(self->m_daemonBus, nullptr, DAEMON_INTERFACE, DAEMON_OBJECT_PATH,
                             "org.freedesktop.DBus.Properties", "Get", &DbusThread::onBlockedAppsReply, self, "(ss)",
                             DAEMON_INTERFACE, "BlockedApps");
    return 0;
}

auto DbusThread::onBlockedAppsReply(sd_bus_message *msg, void *userdata, sd_bus_error * /*unused*/) -> int {
    auto *self = static_cast<DbusThread *>(userdata);
    self->handleBlockedAppsReply(msg);
    self->emitCurrentFocusEvent();
    return 0;
}

auto DbusThread::onBlockedAppsChanged(sd_bus_message *msg, void *userdata, sd_bus_error * /*unused*/) -> int {
    auto *self = static_cast<DbusThread *>(userdata);
    self->handleBlockedAppsChanged(msg);
    return 0;
}

auto DbusThread::onNameOwnerChanged(sd_bus_message *msg, void *userdata, sd_bus_error * /*unused*/) -> int {
    auto *ctx = static_cast<NameOwnerCtx *>(userdata);
    ctx->self->handleNameOwnerChanged(msg, ctx->bus);
    return 0;
}

auto DbusThread::onLogindSignal(sd_bus_message *msg, void *userdata, sd_bus_error * /*unused*/) -> int {
    auto *self = static_cast<DbusThread *>(userdata);
    self->handleLogindSignal(msg);
    return 0;
}

auto DbusThread::onScreenSaverSignal(sd_bus_message *msg, void *userdata, sd_bus_error * /*unused*/) -> int {
    auto *self = static_cast<DbusThread *>(userdata);
    self->handleScreenSaverSignal(msg);
    return 0;
}

auto DbusThread::onGetFocusState(sd_bus_message *msg, void *userdata, sd_bus_error * /*unused*/) -> int {
    auto *self = static_cast<DbusThread *>(userdata);

    // Build the reply payload as a variant containing struct(ussu).
    uint32_t tag = 0;
    // Thread-local buffers for string data (must outlive sd_bus_message_append).
    thread_local std::string g_wClass;
    thread_local std::string g_wTitle;

    if (self->m_focusedApp.has_value()) {
        tag = static_cast<uint32_t>(EventTag::Focus);
        g_wClass = self->m_focusedApp->wclass;
        g_wTitle = self->m_focusedApp->wTitle;
    } else {
        tag = static_cast<uint32_t>(EventTag::Unfocus);
        g_wClass.clear();
        g_wTitle.clear();
    }

    sd_bus_message *reply = nullptr;
    int r = sd_bus_message_new_method_return(msg, &reply);
    if (r < 0) {
        return r;
    }

    r = sd_bus_message_open_container(reply, 'v', "(ussu)");
    if (r < 0) {
        sd_bus_message_unref(reply);
        return r;
    }
    r = sd_bus_message_append(reply, "(ussu)", tag, g_wClass.c_str(), g_wTitle.c_str(), 0U);
    if (r < 0) {
        sd_bus_message_unref(reply);
        return r;
    }
    r = sd_bus_message_close_container(reply);
    if (r < 0) {
        sd_bus_message_unref(reply);
        return r;
    }

    r = sd_bus_send(self->m_daemonBus, reply, nullptr);
    sd_bus_message_unref(reply);
    return r < 0 ? r : 0;
}

// ═════════════════════════════════════════════════════════════════════════════
// Generic bus-selection async reply
// ═════════════════════════════════════════════════════════════════════════════

auto DbusThread::onBusSelectReply(sd_bus_message *msg, void *userdata, sd_bus_error *error) -> int {
    auto *self = static_cast<DbusThread *>(userdata);
    if (error != nullptr && (sd_bus_error_is_set(error) != 0)) {
        self->advanceBusSelection(nullptr);
    } else {
        self->advanceBusSelection(msg);
    }
    return 0;
}

// ═════════════════════════════════════════════════════════════════════════════
// Construction / destruction
// ═════════════════════════════════════════════════════════════════════════════

DbusThread::DbusThread(ThreadChannels &channels) : m_channels(channels), m_sysNameOwnerCtx{}, m_sessNameOwnerCtx{} {
    m_thread = std::thread(&DbusThread::run, this);
}

DbusThread::~DbusThread() {
    if (m_thread.joinable()) {
        m_thread.join();
    }
}

void DbusThread::requestShutdown() { m_shutdownRequested = true; }

void DbusThread::join() {
    if (m_thread.joinable()) {
        m_thread.join();
    }
}

// ═════════════════════════════════════════════════════════════════════════════
// Async bus selection — event-loop driven, no sync D-Bus calls
// ═════════════════════════════════════════════════════════════════════════════

void DbusThread::issueNameHasOwner(sd_bus *bus) {
    if (bus == nullptr) {
        // Bus not available — skip this step immediately.
        advanceBusSelection(nullptr);
        return;
    }
    int r = sd_bus_call_method_async(bus, nullptr, DBUS_INTERFACE, DBUS_OBJECT_PATH, DBUS_INTERFACE,
                                     NAME_HAS_OWNER_METHOD, &DbusThread::onBusSelectReply, this, "s", DAEMON_INTERFACE);
    if (r < 0) {
        advanceBusSelection(nullptr);
    }
}

void DbusThread::issueStartServiceByName(sd_bus *bus) {
    if (bus == nullptr) {
        advanceBusSelection(nullptr);
        return;
    }
    int r = sd_bus_call_method_async(bus, nullptr, DBUS_INTERFACE, DBUS_OBJECT_PATH, DBUS_INTERFACE,
                                     START_SERVICE_BY_NAME_METHOD, &DbusThread::onBusSelectReply, this, "su",
                                     DAEMON_INTERFACE, 0U);
    if (r < 0) {
        advanceBusSelection(nullptr);
    }
}

auto DbusThread::parseNameHasOwner(sd_bus_message *msg) -> bool {
    if (msg == nullptr) {
        return false;
    }
    int hasOwner = 0;
    int r = sd_bus_message_read(msg, "b", &hasOwner);
    return r >= 0 && hasOwner != 0;
}

auto DbusThread::parseStartResult(sd_bus_message *msg) -> bool {
    if (msg == nullptr) {
        return false;
    }
    uint32_t result = 0;
    int r = sd_bus_message_read(msg, "u", &result);
    return r >= 0 && (result == 1U || result == 2U);
}

void DbusThread::advanceBusSelection(sd_bus_message *msg) {
    switch (m_selectStep) {
    case BusSelectStep::CheckSys: {
        if (parseNameHasOwner(msg)) {
            logInfo("DbusThread: daemon found on system bus");
            finishBusSelection(m_sysBus.get());
            return;
        }
        logInfo("DbusThread: no daemon on system bus — trying session bus");
        m_selectStep = BusSelectStep::CheckSess;
        issueNameHasOwner(m_sessBus.get());
        break;
    }
    case BusSelectStep::CheckSess: {
        if (parseNameHasOwner(msg)) {
            logInfo("DbusThread: daemon found on session bus");
            finishBusSelection(m_sessBus.get());
            return;
        }
        logInfo("DbusThread: no daemon on session bus — activating system bus");
        m_selectStep = BusSelectStep::ActivateSys;
        issueStartServiceByName(m_sysBus.get());
        break;
    }
    case BusSelectStep::ActivateSys: {
        if (parseStartResult(msg)) {
            m_selectStep = BusSelectStep::ReCheckSys;
            issueNameHasOwner(m_sysBus.get());
        } else {
            logInfo("DbusThread: system activation failed — trying session");
            m_selectStep = BusSelectStep::ActivateSess;
            issueStartServiceByName(m_sessBus.get());
        }
        break;
    }
    case BusSelectStep::ReCheckSys: {
        if (parseNameHasOwner(msg)) {
            logInfo("DbusThread: daemon activated on system bus");
            finishBusSelection(m_sysBus.get());
        } else {
            logInfo("DbusThread: system activation re-check failed — trying session");
            m_selectStep = BusSelectStep::ActivateSess;
            issueStartServiceByName(m_sessBus.get());
        }
        break;
    }
    case BusSelectStep::ActivateSess: {
        if (parseStartResult(msg)) {
            m_selectStep = BusSelectStep::ReCheckSess;
            issueNameHasOwner(m_sessBus.get());
        } else {
            logInfo("DbusThread: session activation failed — degraded mode");
            finishBusSelection(nullptr);
        }
        break;
    }
    case BusSelectStep::ReCheckSess: {
        if (parseNameHasOwner(msg)) {
            logInfo("DbusThread: daemon activated on session bus");
            finishBusSelection(m_sessBus.get());
        } else {
            logInfo("DbusThread: session activation re-check failed — degraded mode");
            finishBusSelection(nullptr);
        }
        break;
    }
    default:
        break;
    }
}

void DbusThread::finishBusSelection(sd_bus *bus) {
    m_daemonBus = bus;
    m_selectStep = BusSelectStep::Idle;

    if (bus == nullptr) {
        logInfo("DbusThread: daemon unreachable — running in degraded mode");
        m_daemonOwner = false;
        return;
    }

    m_daemonOwner = true;

    // Subscribe to daemon signals on whichever bus the daemon is on.
    subscribeToBlockedApps();

    // Register with the daemon — the sync NameHasOwner check is skipped
    // because our bus selection already confirmed the owner.
    setupDaemonProxy();
}

// ═════════════════════════════════════════════════════════════════════════════
// Thread entry point
// ═════════════════════════════════════════════════════════════════════════════

void DbusThread::run() {
    logInfo("DbusThread: starting");

    // ── Open persistent bus connections (RAII via UniqueBus) ──

    sd_bus *raw = nullptr;
    int r = sd_bus_open_system(&raw);
    if (r < 0) {
        logErr("DbusThread: system bus unavailable");
    }
    m_sysBus.reset(raw);

    raw = nullptr;
    r = sd_bus_open_user(&raw);
    if (r < 0) {
        logErr("DbusThread: session bus unavailable");
    }
    m_sessBus.reset(raw);

    if (m_sysBus == nullptr && m_sessBus == nullptr) {
        logErr("DbusThread: no D-Bus bus available");
        return;
    }

    // ── Event loop setup ──

    sd_event *evRaw = nullptr;
    r = sd_event_default(&evRaw);
    if (r < 0) {
        logErr("DbusThread: sd_event_default failed");
        m_sysBus.reset();
        m_sessBus.reset();
        return;
    }
    m_event.reset(evRaw);

    // Attach both bus connections to the event loop so the event loop
    // can process D-Bus messages (including async replies) on both.
    if (m_sysBus) {
        r = sd_bus_attach_event(m_sysBus.get(), m_event.get(), SD_EVENT_PRIORITY_NORMAL);
        if (r < 0) {
            logErr("DbusThread: sd_bus_attach_event (sys) failed");
        }
    }
    if (m_sessBus) {
        r = sd_bus_attach_event(m_sessBus.get(), m_event.get(), SD_EVENT_PRIORITY_NORMAL);
        if (r < 0) {
            logErr("DbusThread: sd_bus_attach_event (sess) failed");
        }
    }

    // Export Manager interface on both buses so the daemon can call
    // GetFocusState regardless of which bus it connects on.
    static const sd_bus_vtable manager_vtable[] = {
        SD_BUS_VTABLE_START(0),
        SD_BUS_METHOD("GetFocusState", "", "v", &DbusThread::onGetFocusState, SD_BUS_VTABLE_UNPRIVILEGED),
        SD_BUS_SIGNAL("Event", "v", 0),
        SD_BUS_VTABLE_END,
    };

    auto exportVtable = [&](sd_bus *bus, const char *label) -> void {
        if (bus == nullptr) {
            return;
        }
        r = sd_bus_add_object_vtable(bus, nullptr, MANAGER_OBJECT_PATH, MANAGER_INTERFACE, manager_vtable, this);
        if (r < 0) {
            logErr(std::string{"DbusThread: export Manager on "} + label + " failed");
        } else {
            logInfo(std::string{"DbusThread: exported Manager on "} + label);
        }
    };
    exportVtable(m_sysBus.get(), "system bus");
    exportVtable(m_sessBus.get(), "session bus");

    // Compositor eventfd
    {
        sd_event_source *srcRaw = nullptr;
        r = sd_event_add_io(m_event.get(), &srcRaw, m_channels.msgEfd, EPOLLIN, &DbusThread::onCompositorEvent, this);
        if (r < 0) {
            logErr("DbusThread: failed to add compositor eventfd");
        } else {
            m_compositorEventSrc.reset(srcRaw);
        }
    }

    // ── Bus-affine subscriptions (set up immediately) ──
    //
    // logind lives on the system bus; GNOME ScreenSaver on the session bus.
    // NameOwnerChanged is watched on both for cross-bus daemon detection.

    subscribeToLogind(m_sysBus.get());
    subscribeToScreenSaver(m_sessBus.get());
    subscribeNameOwnerChanged(m_sysBus.get());
    subscribeNameOwnerChanged(m_sessBus.get());

    // ── Async bus selection (event-loop driven) ──
    //
    // The 4-step bus selection runs as a chain of async D-Bus method calls
    // so no synchronous call ever blocks the event loop.
    //   Step 1: NameHasOwner on system bus
    //   Step 2: NameHasOwner on session bus
    //   Step 3: StartServiceByName + re-check on system bus
    //   Step 4: StartServiceByName + re-check on session bus

    m_selectStep = BusSelectStep::CheckSys;
    issueNameHasOwner(m_sysBus.get());

    // ── Event loop ──
    logInfo("DbusThread: entering event loop");
    while (!m_shutdownRequested) {
        r = sd_event_run(m_event.get(), UINT64_MAX);
        if (r < 0) {
            logErr("DbusThread: event loop error");
            break;
        }
    }

    logInfo("DbusThread: shutting down");

    // Detach from event loop before destroying resources.
    if (m_sysBus) {
        sd_bus_detach_event(m_sysBus.get());
    }
    if (m_sessBus) {
        sd_bus_detach_event(m_sessBus.get());
    }

    // RAII unique_ptrs handle _unref automatically.
    m_compositorEventSrc.reset();
    m_event.reset();
    m_sysBus.reset();
    m_sessBus.reset();

    // Ack compositor
    eventfd_write(m_channels.ackEfd, 1);
    logInfo("DbusThread: shutdown complete");
}

// ═════════════════════════════════════════════════════════════════════════════
// Compositor message drain
// ═════════════════════════════════════════════════════════════════════════════

void DbusThread::drainCompositorMessages() {
    size_t available = m_channels.msgQueue.get_num_items_ready();
    if (available == 0) {
        return;
    }

    auto scope = m_channels.msgQueue.prepare_read(available);
    auto b1 = scope.get_block1();
    auto b2 = scope.get_block2();

    auto handleFocusUpdate = [this](const FocusUpdate &fu) -> void {
        if (fu.wclass.has_value()) {
            m_focusedApp = DbusFocusState{.wclass = *fu.wclass, .wTitle = fu.wTitle, .blocked = false};
        } else {
            m_focusedApp = std::nullopt;
        }
        emitCurrentFocusEvent();
    };

    auto handleBlockedFocus = [this](const BlockedFocus &bf) -> void {
        m_focusedApp = DbusFocusState{.wclass = bf.wclass, .wTitle = bf.wTitle, .blocked = true};
        emitCurrentFocusEvent();
    };

    auto handleIdleChanged = [this](const IdleChanged &ic) -> void {
        if (!m_registered) {
            return;
        }
        if (m_daemonBus == nullptr) {
            return;
        }
        if (ic.idle) {
            sendDbusSignal(m_daemonBus, static_cast<uint32_t>(EventTag::Idle), "", "", 0);
        } else {
            sendDbusSignal(m_daemonBus, static_cast<uint32_t>(EventTag::Resume), "", "", 0);
        }
    };

    for (auto &item : b1) {
        std::visit(Overloaded{handleFocusUpdate, handleBlockedFocus, handleIdleChanged,
                              [this](const ShutdownMsg &) -> void { m_shutdownRequested = true; }},
                   item);
    }
    for (auto &item : b2) {
        std::visit(Overloaded{handleFocusUpdate, handleBlockedFocus, handleIdleChanged,
                              [this](const ShutdownMsg &) -> void { m_shutdownRequested = true; }},
                   item);
    }
}

// ═════════════════════════════════════════════════════════════════════════════
// BlockedApps property reply
// ═════════════════════════════════════════════════════════════════════════════

void DbusThread::handleBlockedAppsReply(sd_bus_message *msg) {
    SyncAllCmd syncCmd;

    int r = sd_bus_message_enter_container(msg, 'v', nullptr);
    if (r < 0) {
        logErr("handleBlockedAppsReply: enter_container(v) failed");
        return;
    }

    r = sd_bus_message_enter_container(msg, SD_BUS_TYPE_ARRAY, "(sxut)");
    if (r >= 0) {
        while (sd_bus_message_enter_container(msg, SD_BUS_TYPE_STRUCT, "sxut") > 0) {
            const char *wClass = nullptr;
            int64_t policyId = 0;
            uint32_t reason = 0;
            uint64_t blockedSince = 0;

            r = sd_bus_message_read(msg, "sxut", &wClass, &policyId, &reason, &blockedSince);
            if (r < 0) {
                sd_bus_message_exit_container(msg);
                break;
            }

            (void)policyId;
            (void)blockedSince;

            auto wc = WindowClass::from_raw(wClass);
            auto br = raw_to_block_reason(reason);
            if (wc.has_value() && br.has_value()) {
                syncCmd.entries.emplace_back(wc->value(), *br);
            }

            sd_bus_message_exit_container(msg);
        }
        sd_bus_message_exit_container(msg);
    }
    sd_bus_message_exit_container(msg);

    auto ws = m_channels.cmdQueue.prepare_write(1);
    if (ws.get_items_written() > 0) {
        for (auto &slot : ws) {
            slot = syncCmd;
        }
        eventfd_write(m_channels.cmdEfd, 1);
    } else {
        logErr("handleBlockedAppsReply: cmd queue full");
    }

    logInfo("Initial sync: " + std::to_string(syncCmd.entries.size()) + " blocked apps");
}

// ═════════════════════════════════════════════════════════════════════════════
// BlockedAppsChanged signal handler
// ═════════════════════════════════════════════════════════════════════════════

void DbusThread::handleBlockedAppsChanged(sd_bus_message *msg) {
    uint32_t uid = 0;
    const char *rawAppClass = nullptr;
    int blocked = 0;
    uint32_t reason = 0;

    int r = sd_bus_message_read(msg, "usbu", &uid, &rawAppClass, &blocked, &reason);
    if (r < 0) {
        logErr("BlockedAppsChanged: read failed");
        return;
    }

    auto wc = WindowClass::from_raw(rawAppClass);
    if (!wc.has_value()) {
        logErr("BlockedAppsChanged: invalid wclass");
        return;
    }
    auto br = raw_to_block_reason(reason);
    if (!br.has_value()) {
        logErr("BlockedAppsChanged: invalid reason");
        return;
    }

    if (blocked != 0) {
        BlockCmd cmd(wc->value(), *br);
        auto ws = m_channels.cmdQueue.prepare_write(1);
        if (ws.get_items_written() > 0) {
            for (auto &slot : ws) {
                slot = cmd;
            }
            eventfd_write(m_channels.cmdEfd, 1);
        } else {
            logErr("BlockedAppsChanged: cmd queue full");
        }
    } else {
        UnblockCmd cmd{wc->value()};
        auto ws = m_channels.cmdQueue.prepare_write(1);
        if (ws.get_items_written() > 0) {
            for (auto &slot : ws) {
                slot = cmd;
            }
            eventfd_write(m_channels.cmdEfd, 1);
        } else {
            logErr("BlockedAppsChanged: cmd queue full");
        }
    }
}

// ═════════════════════════════════════════════════════════════════════════════
// NameOwnerChanged handler
// ═════════════════════════════════════════════════════════════════════════════

void DbusThread::handleNameOwnerChanged(sd_bus_message *msg, sd_bus *sourceBus) {
    const char *name = nullptr;
    const char *oldOwner = nullptr;
    const char *newOwner = nullptr;
    int r = sd_bus_message_read(msg, "sss", &name, &oldOwner, &newOwner);
    if (r < 0) {
        return;
    }
    if (std::string_view{name} != DAEMON_INTERFACE) {
        return;
    }

    if ((oldOwner != nullptr) && (*oldOwner != 0) && ((newOwner == nullptr) || (*newOwner == 0))) {
        // Disappeared — m_daemonBus stays pointing to the bus so the
        // existing BlockedAppsChanged match remains valid if the daemon
        // reappears on the same bus.
        m_registered = false;
        m_daemonOwner = false;
        logInfo("Daemon disappeared");

        SyncAllCmd cmd{};
        auto ws = m_channels.cmdQueue.prepare_write(1);
        if (ws.get_items_written() > 0) {
            for (auto &slot : ws) {
                slot = cmd;
            }
            eventfd_write(m_channels.cmdEfd, 1);
        }
    } else if (((oldOwner == nullptr) || (*oldOwner == 0)) && (newOwner != nullptr) && (*newOwner != 0)) {
        // Appeared — subscribe on the new bus if the daemon migrated.
        m_daemonOwner = true;
        logInfo("Daemon appeared — registering");

        if (m_registered) {
            return;
        }

        if (m_daemonBus != sourceBus) {
            m_daemonBus = sourceBus;
            subscribeToBlockedApps();
        }

        setupDaemonProxy();
    } else if ((oldOwner != nullptr) && (*oldOwner != 0) && (newOwner != nullptr) && (*newOwner != 0)) {
        // Restarted — same bus, BlockedAppsChanged match is still active
        m_registered = false;
        logInfo("Daemon restarted — re-registering");
        setupDaemonProxy();
    }
}

// ═════════════════════════════════════════════════════════════════════════════
// logind PrepareForSleep / PrepareForShutdown
// ═════════════════════════════════════════════════════════════════════════════

void DbusThread::handleLogindSignal(sd_bus_message *msg) {
    const char *member = sd_bus_message_get_member(msg);
    int value = 0;

    if (std::strcmp(member, "PrepareForSleep") == 0) {
        int r = sd_bus_message_read(msg, "b", &value);
        if (r < 0) {
            return;
        }
        if (value == 0) {
            if (!m_screenLocked && m_focusedApp.has_value()) {
                emitCurrentFocusEvent();
            }
            return;
        }
        emitSystemEvent(EventTag::Power, static_cast<uint32_t>(PowerTag::Suspend));
    } else if (std::strcmp(member, "PrepareForShutdown") == 0) {
        int r = sd_bus_message_read(msg, "b", &value);
        if (r < 0) {
            return;
        }
        if (value != 0) {
            emitSystemEvent(EventTag::Power, static_cast<uint32_t>(PowerTag::Shutdown));
        }
    }
}

// ═════════════════════════════════════════════════════════════════════════════
// GNOME ScreenSaver ActiveChanged
// ═════════════════════════════════════════════════════════════════════════════

void DbusThread::handleScreenSaverSignal(sd_bus_message *msg) {
    int active = 0;
    int r = sd_bus_message_read(msg, "b", &active);
    if (r < 0) {
        return;
    }

    m_screenLocked = (active != 0);
    if (active != 0) {
        emitSystemEvent(EventTag::Locked);
    } else if (m_focusedApp.has_value()) {
        emitCurrentFocusEvent();
    }
}

// ═════════════════════════════════════════════════════════════════════════════
// Daemon proxy & subscriptions
// ═════════════════════════════════════════════════════════════════════════════

auto DbusThread::setupDaemonProxy() -> bool {
    if (m_daemonBus == nullptr) {
        return false;
    }

    // The event guarantees the daemon is present — no sync NameHasOwner.
    // Just issue the async RegisterPlugin call.
    m_daemonOwner = true;
    int r = sd_bus_call_method_async(m_daemonBus, nullptr, DAEMON_INTERFACE, DAEMON_OBJECT_PATH, DAEMON_INTERFACE,
                                     "RegisterPlugin", &DbusThread::onRegisterReply, this, nullptr);
    if (r < 0) {
        logInfo("setupDaemonProxy: RegisterPlugin async call failed");
        return false;
    }
    return true;
}

void DbusThread::subscribeNameOwnerChanged(sd_bus *bus) {
    if (bus == nullptr) {
        return;
    }
    const auto match = std::string{"type='signal',sender='org.freedesktop.DBus',"
                                   "interface='org.freedesktop.DBus',"
                                   "member='NameOwnerChanged',"
                                   "path='/org/freedesktop/DBus'"};
    // Use the member context corresponding to this bus — no heap allocation.
    auto *ctx = (bus == m_sysBus.get()) ? &m_sysNameOwnerCtx : &m_sessNameOwnerCtx;
    ctx->self = this;
    ctx->bus = bus;
    sd_bus_slot *slot = nullptr;
    sd_bus_add_match(bus, &slot, match.c_str(), &DbusThread::onNameOwnerChanged, ctx);
}

void DbusThread::subscribeToBlockedApps() {
    if (m_daemonBus == nullptr) {
        return;
    }
    const auto match = std::string{"type='signal',interface='"} + DAEMON_INTERFACE + "',member='" +
                       BLOCKED_APPS_CHANGED_SIGNAL + "',path='" + DAEMON_OBJECT_PATH + "'";
    sd_bus_add_match(m_daemonBus, nullptr, match.c_str(), &DbusThread::onBlockedAppsChanged, this);
    logInfo("Subscribed to BlockedAppsChanged");
}

void DbusThread::subscribeToLogind(sd_bus *bus) {
    if (bus == nullptr) {
        return;
    }
    const auto match = std::string{"type='signal',interface='org.freedesktop.login1.Manager',"
                                   "path='/org/freedesktop/login1'"};
    sd_bus_add_match(bus, nullptr, match.c_str(), &DbusThread::onLogindSignal, this);
    logInfo("Subscribed to logind");
}

void DbusThread::subscribeToScreenSaver(sd_bus *bus) {
    if (bus == nullptr) {
        return;
    }
    const auto match = std::string{"type='signal',interface='org.gnome.ScreenSaver',"
                                   "member='ActiveChanged',"
                                   "path='/org/gnome/ScreenSaver'"};
    sd_bus_add_match(bus, nullptr, match.c_str(), &DbusThread::onScreenSaverSignal, this);
    logInfo("Subscribed to GNOME ScreenSaver");
}

// ═════════════════════════════════════════════════════════════════════════════
// Signal emission
// ═════════════════════════════════════════════════════════════════════════════

auto DbusThread::canEmit() const -> bool { return m_registered && (m_daemonBus != nullptr); }

void DbusThread::emitRawEvent(uint32_t tag, const std::string &wClass, const std::string &wTitle, uint32_t powerTag) {
    if (!canEmit()) {
        return;
    }
    sendDbusSignal(m_daemonBus, tag, wClass.c_str(), wTitle.c_str(), powerTag);
}

void DbusThread::emitCurrentFocusEvent() {
    if (!canEmit()) {
        return;
    }

    if (!m_focusedApp.has_value()) {
        sendDbusSignal(m_daemonBus, static_cast<uint32_t>(EventTag::Unfocus), "", "", 0);
        return;
    }
    auto &fa = *m_focusedApp;
    auto tag = fa.blocked ? EventTag::Block : EventTag::Focus;
    sendDbusSignal(m_daemonBus, static_cast<uint32_t>(tag), fa.wclass.c_str(), fa.wTitle.c_str(), 0);
}

void DbusThread::emitSystemEvent(EventTag tag, uint32_t powerTag) {
    if (!canEmit()) {
        return;
    }
    sendDbusSignal(m_daemonBus, static_cast<uint32_t>(tag), "", "", powerTag);
}

} // namespace wellbeing
