# The Platform Trait

The central OS abstraction. Defined in `platform/mod.rs`. See the
[overview](./README.md) for where it fits in the two-binary split. The Linux
implementation lives in [03-linux-platform.md](./03-linux-platform.md); the
plugin D-Bus contract it talks to is in [04-plugin-ipc.md](./04-plugin-ipc.md).

## The Platform Trait

The Platform trait defines operations the daemon needs from the OS — primarily
event ingestion and user notification. Blocking overlay management is handled
declaratively: the daemon writes block state to `ActiveBlocks` on its own D-Bus
interface, and the compositor plugin reads that state directly. See
[04-plugin-ipc.md](./04-plugin-ipc.md) for the full IPC architecture.

```
pub trait Platform: Send + Sync + 'static {
    type EventStream: Stream<Item = PlatformEvent> + Send + 'static;

    /// Send a desktop notification via org.freedesktop.Notifications.
    /// Used by Notify policies to alert the user when a time limit is exceeded.
    /// Non-blocking — fire-and-forget from the caller's perspective.
    async fn notify(&self, title: &str, body: &str) -> Result<()>;
}
```

The trait is a pure operation contract — no constructor. Each impl provides its
own builder/factory that guarantees full initialization before any operation is
accessible.

### Construction — Per-Platform Builders

The Platform trait does **not** define constructors. Each platform impl provides
its own builder or factory function with its required parameters encoded in
`new()`. This prevents the API error of calling operations on an uninitialized
platform.

**LinuxPlatformBuilder** has no compositor-specific state — the daemon
communicates with whatever compositor plugin is registered on the daemon's D-Bus
bus. No detection, no feature gates for compositor variants. The builder
connects to D-Bus and returns the platform with an event stream; the plugin is
discovered asynchronously via `NameOwnerChanged`.

**MockPlatform** has no builder — its constructor is infallible and takes
pre-seeded event data directly. `notify()` is a no-op.

**Interior mutability:** All `&self` Platform methods imply the impl uses
interior mutability (`Arc<Mutex<...>>`, `Atomic*`, or `RefCell`). This is by
design — the platform handle is shared across multiple actors.

### Concurrency Model

The daemon uses `&self` on the Platform trait (not `&mut self`), but the Linux
impl's mutable state (D-Bus connection, plugin proxy) is behind interior
mutability. The `Platform` impl is concrete and known at compile time — actors
are generic over `P: Platform`.

Block state management flows through the daemon's `ActiveBlocks` state (exposed
on the D-Bus `org.wellbeing.v1.Controller` interface), not through Platform. The
EnforcerActor writes block state via an internal channel or shared state; the
plugin reads the D-Bus property independently.

### Event Model

Platform events are the sole input to the system state machine. No platform
knowledge leaks beyond `PlatformEvent`.

| Event           | Fields                                     | Source                                                      | Consumer                                                         |
| --------------- | ------------------------------------------ | ----------------------------------------------------------- | ---------------------------------------------------------------- |
| `WindowFocused` | `{app_id, title, pid, uid, overlay_shown}` | Plugin `FocusChanged` signal                                | EnforcerActor (policy evaluation), TrackerActor (session timing) |
| `Unfocused`     | —                                          | Plugin `FocusChanged` signal (Desktop variant)              | EnforcerActor (close interval)                                   |
| `Idle`          | —                                          | Plugin `ActivityChanged` signal (FocusActivityTag::Idle)    | EnforcerActor (pause interval)                                   |
| `Resumed`       | —                                          | Plugin `ActivityChanged` signal (FocusActivityTag::Resumed) | EnforcerActor (resume interval)                                  |
| `UserAction`    | `{app_id, action}`                         | Plugin `UserAction` signal                                  | EnforcerActor (grant extension / close overlay)                  |

`Locked`, `LoggedOut`, `Slept`, and `ShutDown` are NOT `PlatformEvent` variants.
They are emitted directly into the event log by the session / power watcher
(`platform/linux/suspend.rs`) from systemd-logind signals — bypassing the
enforcer gate because they are terminal and need no policy evaluation. They
carry no `app_id` and simply close the open interval.

**`overlay_shown` flag:** The compositor plugin includes this boolean in every
`WindowFocused` event, indicating whether a block overlay is already rendered on
the focused window. Used for diagnostics and dashboards. Crash recovery is
handled by the plugin reading `ActiveBlocks` on reconnect.

**`UserAction` fields:** The plugin sends only `app_id` + `action`. The daemon
looks up the corresponding `policy_id` from its own `ActiveBlocks` state.

**Synthetic events:** When the user grants extra time after a block, the
EnforcerActor inserts a synthetic `WindowFocused` event after writing the
extension. This opens a new focus interval, ensuring duration calculations
reflect actual post-grant usage.

## References

- [04-plugin-ipc.md](./04-plugin-ipc.md) — declarative plugin IPC,
  `ActiveBlocks`
- [03-linux-platform.md](./03-linux-platform.md) — Linux Platform impl
- [06-daemon-dbus.md](./06-daemon-dbus.md) — `ActiveBlocks` property on daemon
  interface
