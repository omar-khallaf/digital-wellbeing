# Deployment Modes — System Daemon vs User Session Daemon

The daemon supports two runtime modes. Mode is selected at startup from the
effective uid, overridable by CLI flags. This document owns the bus/scope
selection, the GUI client resolution algorithm, the plugin resolution rule, and
the degraded-mode behavior. The D-Bus interface itself is unchanged
([06-daemon-dbus.md](./06-daemon-dbus.md)); RBAC is covered in
[07-rbac.md](./07-rbac.md); deployment artifacts in
[10-deployment.md](./10-deployment.md).

## Modes

| Mode    | Selected when | Bus for org.wellbeing.v1.Controller | Database path                              | Scope       | RBAC                        |
| ------- | ------------- | ----------------------------------- | ------------------------------------------ | ----------- | --------------------------- |
| System  | uid == 0      | system bus                          | /var/lib/digital-wellbeing/db.sqlite (600) | Multi-user  | Full root-vs-user matrix    |
| Session | uid > 0       | session bus                         | $XDG_DATA_HOME/digital-wellbeing/db.sqlite | Single-user | Pass-through (own uid only) |

Both modes expose the identical org.wellbeing.v1.Controller interface and the
same org.wellbeing.v1.Manager plugin contract. Clients and plugins are
bus-agnostic: they listen on both busses simultaneously and use a 4-step
preference algorithm to select which bus hosts the daemon (below), never
hardcode it. This dual-bus design enables recovery from daemon restarts even if
the daemon reappears on a different bus.

### Why a session mode?

The original design assumes a root-owned system daemon enforcing wellbeing for
all users on the device. That requires root install + a systemd system service +
a root-owned state directory. A user who cannot (or does not want to) install as
root should still be able to run wellbeing for themselves only. Session mode
makes the daemon enforce exactly one user — the one it runs as — and claim its
well-known name on that user's session bus. This matches the device-local,
per-user nature of wellbeing and removes the root requirement for single-user
use.

## Mode Selection

The daemon's operating mode is resolved once at startup, before any D-Bus name
is claimed. Two variants are possible: System, which uses the system bus and
stores its database at /var/lib/digital-wellbeing/db.sqlite; and Session, which
uses the session bus, stores its database under
$XDG_DATA_HOME/digital-wellbeing/db.sqlite, and carries the daemon's own uid.

Resolution follows a two-tier rule. If the user explicitly passes a --bus flag
(system or session), the corresponding mode is used directly. Otherwise the
effective UID decides — a root process enters System mode, any non-root process
enters Session mode with the real uid attached.

The resolved mode value is constructed once in main.rs and threaded into the
store builder, the D-Bus server (which determines which bus to connect to), and
the DaemonScope used by RBAC. There is no runtime re-detection — the type makes
the mode unrepresentable as "unknown".

### logind stays on the system bus

A session-mode daemon still needs power/session state (Slept/ShutDown/
Locked/LoggedOut). systemd-logind lives on the system bus and is reachable by
any user, so PowerStateWatcher ([03-linux-platform.md](./03-linux-platform.md))
always connects to the system bus regardless of daemon mode. Only the daemon's
own org.wellbeing.v1.Controller name and the plugin RegisterPlugin target move
to the session bus. A session-mode daemon therefore holds two connections:

- session bus — owns org.wellbeing.v1.Controller, talks to the plugin.
- system bus — logind only.

## Daemon Scope (RBAC)

The daemon mode drives a DaemonScope value used by RBAC branching in the D-Bus
handlers. In System mode the scope is MultiUser — the full root-vs-user
permission matrix applies and any uid's policy rows may be present. In Session
mode the scope is SingleUser, carrying the daemon's own uid.

In SingleUser scope the caller is always the daemon's own user, so the RBAC
matrix collapses to pass-through:

- owner_id / created_by are always written with the daemon's uid.
- All policy CRUD and usage queries are permitted (equivalent to the "root" row
  of the [07-rbac.md](./07-rbac.md) matrix), but scoped to the single user's
  rows — the daemon never holds another user's data.
- The ListPolicies filter_owner argument is ignored (there is only one owner).
- SO_PEERCRED authentication is still performed (it is free and guards against a
  different local user reaching the session bus), but the authorization step
  short-circuits to "allow".

## GUI Daemon Resolution (client side)

The GUI connects to both system and session D-Bus busses simultaneously at
startup. It holds both connections for the lifetime of the process and selects
which one hosts the daemon via a 4-step algorithm that prefers the system bus.
The result is a connection status of either connected (with the chosen bus type)
or disconnected.

The 4-step selection run at startup:

1. System bus already has the daemon? If org.wellbeing.v1.Controller has an
   owner on the system bus, use the system connection.
2. Session bus already has the daemon? If the name has an owner on the session
   bus, use the session connection.
3. Activate the system daemon. Call StartServiceByName on the system bus to
   trigger D-Bus activation of the root systemd service. If the name appears in
   response, use the system connection.
4. Activate the session daemon. Call StartServiceByName on the session bus to
   activate the user service. If the name appears, use the session connection.
5. All four steps failed. The GUI enters degraded mode.

name_owner_present calls org.freedesktop.DBus.NameHasOwner;
start_service_by_name calls org.freedesktop.DBus.StartServiceByName (which
triggers D-Bus activation — see [10-deployment.md](./10-deployment.md)). The
proxy is built against whichever connection resolved. The reverse-registration
pattern is unchanged: the plugin still calls Controller.RegisterPlugin.

### Degraded mode

If all four steps fail, the GUI still holds connections to both busses but sets
a disconnected state. It does not crash or block. It:

1. Shows a status indicator in the navigation drawer showing "Disconnected" with
   a red dot — replacing the old full-width warning banner.
2. Opens normally; dashboards/reports render whatever is cached (likely empty).
3. Disables policy mutations and any action requiring the daemon.

The background refresh loop continues to run every 5 seconds; on each tick it
re-runs the 4-step selection. Once the daemon appears on either bus, the status
updates to connected and normal operation resumes.

This resolves
[open question #1](./12-open-questions.md#1-gui-startup-when-daemon-is-not-running).

## Plugin Resolution

The compositor plugin runs inside the user's session. Unlike the old design
(which probed both busses ephemerally and connected to only one), the plugin now
connects to both busses permanently at startup and holds both connections for
the lifetime of the plugin. The org.wellbeing.v1.Manager interface is registered
on both connections so the daemon can reach the plugin regardless of which bus
it lives on.

The daemon proxy is built against whichever bus the 4-step resolution selects
(system preferred):

1. System bus has org.wellbeing.v1.Controller? -> use system connection.
2. Else session bus has it? -> use session connection.
3. Else StartServiceByName on the system bus -> use system connection.
4. Else StartServiceByName on the session bus -> use session connection.
5. All fail -> degraded mode (plugin still connected to both busses).

This guarantees exactly one enforcing daemon per user: if a root system daemon
is active, both GUI and plugin use it (multi-user enforcement); if not, both use
the user's session daemon (single-user enforcement). Dual-bus listening does not
cause double enforcement — only the selected active daemon proxy communicates
with the daemon; the second connection merely listens for NameOwnerChanged
signals.

### Dual-Bus Recovery

The plugin registers NameOwnerChanged watchers on both connections. When the
daemon disappears from the active bus (crashes, restarts), the watcher fires and
the plugin:

1. Sets the active daemon to "none".
2. Re-runs the 4-step resolution against both held connections.
3. If the daemon reappeared on the other bus (e.g., system daemon crashed and
   session daemon started), creates a fresh daemon proxy on that bus.
4. Calls RegisterPlugin, reads ActiveBlocks, subscribes to signals.

No background polling thread is needed — the dual-bus NameOwnerChanged watchers
provide event-driven recovery. The daemon can restart on a different bus without
the plugin needing to reconnect to D-Bus itself.

### C++ Side: Resolution and Dual Connections

The plugin (Hyprland, sdbus-c++) creates two permanent connections at startup
and uses an instance method to select the active daemon bus. A DaemonBus enum
with variants None, System, and Session replaces the old single-bus variant. The
WellbeingManager class holds both connections, two registered objects (one per
bus), a single active daemon proxy, and the active bus enumeration.

The resolution method implements the same 4-step algorithm: check if the daemon
name is owned on the system bus (step 1) or session bus (step 2), then attempt
to activate it on the system bus (step 3) or session bus (step 4). It returns
the resolved bus or DaemonBus::None if all steps fail.

The probe helpers (nameHasOwner, startServiceByName) are identical to the old
design, but they now operate on permanent connections rather than ephemeral
probe connections. The old ephemeral probe connection helper is removed.

### Plugin lifecycle

PLUGIN_INIT:

1. Create PluginState
2. Create LockManager
3. Connect to BOTH busses permanently
4. Construct WellbeingManager with both connections a. Register Manager
   interface on both connections b. Run resolveActiveDaemonBus() c. If daemon
   found, create daemon proxy, RegisterPlugin, read ActiveBlocks d. Subscribe to
   NameOwnerChanged on both connections e. Enter event loop on both connections
5. Register compositor hooks

PLUGIN_EXIT:

1. Stop event loops
2. Destroy WellbeingManager (drops both connections)
3. Destroy PluginState

## Deploy Artifacts

See [10-deployment.md](./10-deployment.md) for the full file tree. Summary of
the session-mode additions:

| Artifact                                          | Mode    | Notes                                                                                                                      |
| ------------------------------------------------- | ------- | -------------------------------------------------------------------------------------------------------------------------- |
| digital-wellbeing-daemon.service (systemd)        | System  | User=root, BusName=org.wellbeing.v1.Controller (unchanged).                                                                |
| dbus-1/system-services/...Controller.service      | System  | User=root, activates the system daemon.                                                                                    |
| dbus-1/system.d/org.wellbeing.v1.Controller.conf  | System  | Root own; default send/receive (unchanged).                                                                                |
| dbus-1/services/...Controller.service             | Session | Session activation; User omitted (current user).                                                                           |
| ~/.config/systemd/user/...daemon.service          | Session | Optional user systemd unit for autostart.                                                                                  |
| dbus-1/session.d/org.wellbeing.v1.Controller.conf | Session | Usually unnecessary — the session bus lets the owning user own any name by default. Add only if stricter policy is wanted. |
| /var/lib/digital-wellbeing/db.sqlite              | System  | mode 600, root-owned (unchanged).                                                                                          |
| $XDG_DATA_HOME/digital-wellbeing/db.sqlite        | Session | mode 600, user-owned.                                                                                                      |

## Security Notes

- No request signing needed. The plugin reads block state from the daemon's own
  D-Bus name (org.wellbeing.v1.Controller.ActiveBlocks), which is protected by
  the D-Bus daemon. The plugin never accepts commands, so there is no command
  method to spoof. See [05-daemon-auth.md](./05-daemon-auth.md).
- Session bus is per-user. On the session bus only the owning user can claim
  names, so the already-open own policy is naturally tighter than on the system
  bus.
- No cross-user enforcement in session mode — by design. A session daemon cannot
  see or enforce other users; that capability is reserved for the system daemon
  (root).
- SO_PEERCRED still checked even in session mode (cheap, and blocks a different
  local user from reaching the session bus), but authorization is pass-through.
- Dual-bus listening is safe. The plugin holds two anonymous connections but
  only communicates with the daemon through one (the active bus). The second
  connection is purely for NameOwnerChanged signal reception. No duplicate
  enforcement or conflicting state can arise — only the active daemon proxy
  reads ActiveBlocks or sends signals. If both a system daemon and a session
  daemon were running simultaneously, the 4-step algorithm selects the system
  daemon (step 1), exactly as the old single-bus design would have done.

## References

- [06-daemon-dbus.md](./06-daemon-dbus.md) — interface, unchanged; GUI proxy bus
  resolution.
- [07-rbac.md](./07-rbac.md) — DaemonScope / SingleUser pass-through.
- [10-deployment.md](./10-deployment.md) — service files, DB paths, policies.
- [04-plugin-ipc.md](./04-plugin-ipc.md) — plugin resolution mirrors GUI.
- [03-linux-platform.md](./03-linux-platform.md) — logind on system bus in both
  modes.
- [05-daemon-auth.md](./05-daemon-auth.md) — signing unchanged across buses.
- [12-open-questions.md](./12-open-questions.md) — #1 resolved (degraded mode).
