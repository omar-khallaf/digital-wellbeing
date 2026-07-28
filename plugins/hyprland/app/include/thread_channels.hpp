#pragma once

// =============================================================================
// ThreadChannels — lock-free cross-thread channels between compositor and D-Bus.
//
// Holds two SPSC queues (chan B: D-Bus→compositor, chan C: compositor→D-Bus)
// and the eventfds used to wake each thread. Lives in PluginState, shared by
// both threads.
// =============================================================================

#include <array>
#include <semaphore>

#include <LockFreeSpscQueue.h>

#include "messages.hpp"

namespace wellbeing {

/// Total capacity for chan B (D-Bus → compositor commands).
/// Blocked app changes are rare; 64 slots is generous.
inline constexpr size_t CHAN_B_CAPACITY = 64;

/// Total capacity for chan C (compositor → D-Bus messages).
/// Focus events can be rapid during alt-tab; 2048 slots at ~80 bytes each
/// = ~160 KB, provides ~7s of headroom at 144fps before overflow.
inline constexpr size_t CHAN_C_CAPACITY = 2048;

/// Cross-thread communication channels and synchronization primitives.
/// Shared between PluginState (compositor side) and DbusThread (D-Bus side).
struct ThreadChannels {
    // ── Chan B: D-Bus thread writes commands, compositor reads ──
    std::array<CompositorCommand, CHAN_B_CAPACITY> cmdBuf;
    LockFreeSpscQueue<CompositorCommand> cmdQueue;

    // ── Chan C: compositor writes messages, D-Bus thread reads ──
    std::array<DbusMessage, CHAN_C_CAPACITY> msgBuf;
    LockFreeSpscQueue<DbusMessage> msgQueue;

    // ── Eventfds for cross-thread wakeup ──
    int cmdEfd = -1; // D-Bus thread → compositor (registered w/ wl_event_loop)
    int msgEfd = -1; // compositor → D-Bus thread (registered w/ sd_event)
    int ackEfd = -1; // D-Bus thread → compositor shutdown ack

    // ── Shutdown semaphore ──
    std::binary_semaphore shutdownSem{0};

    ThreadChannels() : cmdQueue(cmdBuf), msgQueue(msgBuf) {}
};

} // namespace wellbeing
