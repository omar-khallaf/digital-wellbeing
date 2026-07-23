# Linux Platform

The Linux implementation lives in `platform/linux/`. On Linux, window events
come from the compositor plugin via D-Bus FocusChanged signals. The plugin D-Bus
contract is documented in [04-plugin-ipc.md](./04-plugin-ipc.md); the Platform
trait it implements is in [02-platform.md](./02-platform.md).

## Directory Structure

platform/linux/ ├── mod.rs # impl Platform for Linux ├── manager.rs # D-Bus
ManagerClient — plugin registration, signal subscriptions └── suspend.rs #
systemd-logind D-Bus (PrepareForSleep/Shutdown)

## App Metadata Resolution

App display name and icon are resolved from the app_categories table (see
[persistence/01-database.md](../persistence/01-database.md)). By default, app_id
is used as the display name. The display_name and icon_path columns in
app_categories allow per-app overrides configured by the user or seeded as
defaults.

The categorizer resolution chain is:

1. app_categories (DB — seeded defaults + user edits)
2. AI classification (unmapped apps)
3. Uncategorized

## Power & Session State Handling

When the system is about to suspend, hibernate, shut down, lock, or end a
session, the open focus interval must be closed so wall-clock time during that
state is not counted against the app limit. The Linux platform handles this via
D-Bus integration with systemd-logind (platform/linux/suspend.rs). It emits real
events — never a synthetic Unfocused:

- PrepareForSleep(TRUE) → Slept (covers both suspend and hibernate — logind
  cannot distinguish them at signal time)
- PrepareForShutdown(TRUE) → ShutDown
- Session Lock signal → Locked
- Session removed → LoggedOut

Slept/ShutDown/Locked/LoggedOut are close events: they credit the active
interval via `accumulate_daily_usage` and clear the in-memory FocusState. They
carry no app_id.

Flow (suspend/shutdown example; lock/logout are analogous):

logind signal (PrepareForSleep / PrepareForShutdown = TRUE) │ ▼
PowerStateWatcher (platform/linux/suspend.rs) │ │ INSERT Slept / ShutDown into
events table │ (accumulate_daily_usage credits elapsed time, clears focus state)
▼ Dropped inhibitor → power state change proceeds

| State     | Signal                   | Event emitted |
| --------- | ------------------------ | ------------- |
| Suspend   | PrepareForSleep(TRUE)    | Slept         |
| Hibernate | PrepareForSleep(TRUE)    | Slept         |
| Shutdown  | PrepareForShutdown(TRUE) | ShutDown      |
| Lock      | Session Lock             | Locked        |
| Logout    | Session removed          | LoggedOut     |

The delay mode gives a few seconds to flush before the system pauses. LoggedOut
is also emitted by the SIGTERM/SIGHUP handler on daemon termination (see
[persistence/01-database.md](../persistence/01-database.md)).

Error handling: if the DB flush fails, log the error and release the inhibitor
anyway. Losing a few seconds of usage data is acceptable; blocking a power state
change is not.

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
