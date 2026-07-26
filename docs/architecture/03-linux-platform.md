# Linux Platform

The Linux implementation lives in `platform/linux/`. On Linux, window events
come from the compositor plugin via the unified D-Bus Event signal. The plugin
D-Bus contract is documented in [04-plugin-ipc.md](./04-plugin-ipc.md); the
Platform trait it implements is in [02-platform.md](./02-platform.md).

## App Metadata Resolution

App metadata (display name, icon) is resolved in the daemon's categorization
module, not in the Platform trait. `PlatformEvent` carries only `app_class`,
`title`, `pid`, and `uid`. The categorizer resolution chain is:

1. `app_categories` DB (seeded defaults + user edits)
2. In-memory cache (60s TTL for AI-classified apps)
3. AI classification (unmapped apps)
4. Uncategorized

## Power & Session State Handling

When the system is about to suspend, hibernate, shut down, lock, or end a
session, the open focus interval must be closed so wall-clock time during that
state is not counted against the app limit. The compositor plugin handles this
via its own system signal watchers (`system_watcher.cpp`), emitting unified
Event signals over D-Bus to the daemon. The daemon treats these as ordinary
events — no special power-state handling in the daemon itself.

The plugin subscribes to logind and GNOME ScreenSaver on the appropriate buses:

| State     | Source signal                   | Plugin emits                            |
| --------- | ------------------------------- | --------------------------------------- |
| Suspend   | logind PrepareForSleep(TRUE)    | `Event(EventTag::Power, ..., Suspend)`  |
| Hibernate | logind PrepareForSleep(TRUE)    | `Event(EventTag::Power, ..., Suspend)`  |
| Shutdown  | logind PrepareForShutdown(TRUE) | `Event(EventTag::Power, ..., Shutdown)` |
| Lock      | ScreenSaver ActiveChanged(TRUE) | `Event(EventTag::Locked, ...)`          |
| Logout    | logind Session removed          | `Event(EventTag::LogOut, ...)`          |

Plugin-side flow (suspend example; others analogous):

logind signal PrepareForSleep(TRUE) │ ▼ Plugin `handlePrepareForSleep(true)` →
emits Event signal │ ▼ Daemon receives Event from Manager interface → buffers in
EventBuffer │ ▼ EnforcerActor handles close event on next buffer flush → credits
interval

On daemon shutdown (SIGTERM/SIGHUP), `logind.rs`'s `take_shutdown_inhibit`
creates a logind delay inhibitor to allow the final buffer flush to complete
before the system proceeds (see
[persistence/01-database.md](../persistence/01-database.md)).

### Resume & Screen Unlock

The daemon does NOT handle resume or unlock directly. The compositor plugin is
responsible for syncing focus state after power-state changes. The plugin tracks
screen lock state internally (`m_screenLocked` in `system_watcher.cpp`):

- **Resume while screen unlocked** — `handlePrepareForSleep(false)` emits the
  current focus state via `emitFocusEvent()` if the screen is already unlocked,
  resuming the focus interval.
- **Resume while screen locked** — plugin defers and waits for the unlock
  handler.
- **Screen unlock** — `handleScreenSaverActive(false)` sets
  `m_screenLocked = false` and emits the current focus state via
  `emitFocusEvent()`.

In all cases the plugin emits a standard `Event` signal — either `Focus`,
`Block`, or `Unfocus` depending on what window was focused — and the daemon
processes it like any other event. No synthetic events or DB queries are
involved on the daemon side.

## Compositor Support

| Compositor  | Plugin                | D-Bus Implementation   | Status    |
| ----------- | --------------------- | ---------------------- | --------- |
| Hyprland    | wellbeing-lockdown.so | sdbus-cpp in C++       | v1 target |
| KWin        | wellbeing-effect      | KWin::Effect + D-Bus   | Roadmap   |
| Wayfire     | wellbeing-plugin      | Wayfire plugin + D-Bus | Roadmap   |
| GNOME Shell | wellbeing-extension   | GJS + D-Bus            | Roadmap   |

All compositors implement the same org.wellbeing.v1.Manager D-Bus interface at
the same object path (/org/wellbeing/Manager), but each plugin instance connects
anonymously — the bus daemon assigns a unique bus name (:1.xxx). Discovery is
reverse: at startup each plugin calls Controller.RegisterPlugin(), so the daemon
learns the caller's real uid (via SO_PEERCRED) and unique bus name (from
header.sender()). The daemon does not probe a single org.wellbeing.v1.Manager
name; it watches NameOwnerChanged for each registered plugin's unique bus name
to detect connect/disconnect (see
[04-plugin-ipc.md](./04-plugin-ipc.md#multi-instance-plugin-support)).
