# The Platform Trait

The central OS abstraction. Defined in `platform/mod.rs`. See the
[overview](./README.md) for where it fits in the two-binary split. The Linux
implementation lives in [03-linux-platform.md](./03-linux-platform.md); the
plugin D-Bus contract it talks to is in [04-plugin-ipc.md](./04-plugin-ipc.md).

## The Platform Trait

The Platform trait defines operations the daemon needs from the OS — primarily
event ingestion and user notification. Blocking overlay management is handled
declaratively: the daemon writes block state to ActiveBlocks on its own D-Bus
interface, and the compositor plugin reads that state directly. See
[04-plugin-ipc.md](./04-plugin-ipc.md) for the full IPC architecture.

The trait defines a single associated type for the event stream and one async
method for desktop notifications. It carries no constructor — each platform
implementation provides its own builder that guarantees full initialization
before any operation is accessible.

### Construction — Per-Platform Builders

The Platform trait does not define constructors. Each platform impl provides its
own builder or factory function with required parameters encoded in new(). This
prevents calling operations on an uninitialized platform.

LinuxPlatformBuilder has no compositor-specific state — the daemon communicates
with whatever compositor plugin is registered on the daemon's D-Bus bus. No
detection, no feature gates for compositor variants. The builder connects to
D-Bus and returns the platform with an event stream; the plugin is discovered
asynchronously via NameOwnerChanged.

MockPlatform has no builder — its constructor is infallible and takes pre-seeded
event data directly. The notify method is a no-op.

### Concurrency Model

The daemon uses &self on the Platform trait (not &mut self), but the Linux
impl's mutable state (D-Bus connection, plugin proxy) is behind interior
mutability. The Platform impl is concrete and known at compile time — actors are
generic over P: Platform.

Block state management flows through the daemon's ActiveBlocks state (exposed on
the D-Bus org.wellbeing.v1.Controller interface), not through Platform. The
EnforcerActor writes block state via an internal channel or shared state; the
plugin reads the D-Bus property independently.

### Event Model

Platform events are the sole input to the system state machine. No platform
knowledge leaks beyond PlatformEvent.

| Event            | Fields                           | Source                                                                                             | Consumer                                             |
| ---------------- | -------------------------------- | -------------------------------------------------------------------------------------------------- | ---------------------------------------------------- |
| WindowFocused    | {app_id, title, pid, uid}        | Plugin FocusChanged signal (tag=1)                                                                | EnforcerActor (policy evaluation, interval tracking) |
| WindowBlocked    | {app_id, title, uid}             | Plugin FocusChanged signal (tag=2)                                                                | EnforcerActor (close interval, blocked state)        |
| Unfocused        | —                                | Plugin FocusChanged signal (Desktop variant)                                                      | EnforcerActor (close interval)                       |
| Idle             | —                                | Plugin ActivityChanged signal (FocusActivityTag::Idle)                                            | EnforcerActor (pause interval)                       |
| Resumed          | —                                | Plugin ActivityChanged signal (FocusActivityTag::Resumed)                                         | EnforcerActor (resume interval)                      |
| Slept            | —                                | logind PrepareForSleep(TRUE)                                                                      | EnforcerActor (close interval)                       |
| Locked           | —                                | logind Session Lock                                                                               | EnforcerActor (close interval)                       |
| LoggedOut        | —                                | logind Session removed / SIGTERM                                                                  | EnforcerActor (close interval)                       |
| ShutDown         | —                                | logind PrepareForShutdown(TRUE)                                                                   | EnforcerActor (close interval)                       |
| ResumedSystem    | —                                | logind PrepareForSleep(FALSE) — no-op in enforcer, resync handled by main.rs                      | —                                                    |

WindowFocused (tag=1) is emitted when the user focuses an unblocked window.
WindowBlocked (tag=2) is emitted when the focused window has an active overlay
(the compositor shows the block screen). The plugin decides the tag by checking
`LockManager::isOverlayShown()` at emit time. The variant tag eliminates the
need for a separate boolean — the distinction is encoded in the variant itself.

Unfocused carries no app_id — it closes the open interval without opening a new
one. Slept, Locked, LoggedOut, and ShutDown are also close events: they credit
the active interval and clear the in-memory current_focus map.

Close actions are handled locally in the plugin — the daemon never receives
a user-action event over D-Bus. Block resolution is purely a plugin concern.

Synthetic events: after a block is resolved, the EnforcerActor may insert a
synthetic WindowFocused event if the app was given a new focus interval,
ensuring duration calculations reflect actual post-block usage.

## References

- [04-plugin-ipc.md](./04-plugin-ipc.md) — declarative plugin IPC, ActiveBlocks
- [03-linux-platform.md](./03-linux-platform.md) — Linux Platform impl
- [06-daemon-dbus.md](./06-daemon-dbus.md) — ActiveBlocks property on daemon
  interface
