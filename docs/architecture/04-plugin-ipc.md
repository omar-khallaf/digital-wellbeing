# Plugin IPC (D-Bus)

The daemon and GUI communicate with the compositor plugin over the daemon's bus
— the system bus when the daemon runs in system mode (root), the session bus
when it runs in session mode (non-root). See
[13-deployment-modes.md](./13-deployment-modes.md) for bus/scope selection.

The architecture is declarative: the daemon exposes its block state as a
readable data source, and the plugin reads that state to decide when to show or
hide overlays. The daemon never commands the plugin — it only publishes state.

Plugin bus resolution uses the same 4-step algorithm as the GUI
([13-deployment-modes.md](./13-deployment-modes.md#plugin-resolution)): the
plugin connects to both system and session busses permanently, then selects
which connection hosts the daemon (system present -> session present -> activate
system -> activate session). This guarantees exactly one enforcing daemon per
user while enabling cross-bus daemon restart recovery.

No compositor detection, no socket path configuration, no feature gates.

## D-Bus Interface — org.wellbeing.v1.Manager

The plugin exposes a single interface with signals and a property. It has no
method for the daemon to call — the daemon never commands the plugin. The plugin
is a pure producer of window-domain facts (focus, activity, user clicks) and a
consumer of daemon block state.

Signals (plugin -> daemon):

| Signal          | Payload                                                                         | When                                     |
| --------------- | ------------------------------------------------------------------------------- | ---------------------------------------- |
| FocusChanged    | v — variant: 1 (Desktop) or 2 + struct {app_id, title, pid, uid, overlay_shown} | On every compositor focus switch         |
| ActivityChanged | FocusActivityTag — Idle=0, Resumed=1                                            | User idle state changes                  |
| UserAction      | {app_id, action}                                                                | User presses a button on a block overlay |

Property (readable):

| Property     | Type | Returns                                                  |
| ------------ | ---- | -------------------------------------------------------- |
| CurrentFocus | v    | Same variant as FocusChanged — queryable source of truth |

UserAction simplified: the daemon is the authority on which policy is blocking
which app. When UserAction arrives, the daemon looks up the active block state
for app_id and derives policy_id from its own records — the plugin does not echo
back a signed token.

### CurrentFocus property

D-Bus signals are fire-and-forget — they do not persist their last value, so a
GUI that subscribes after the fact misses the current state. CurrentFocus is a
readable D-Bus property that returns the same variant as the FocusChanged
signal, giving clients a queryable, always-current source of truth on startup.
The signal remains useful as a lightweight change notification.

## Declarative Block State — org.wellbeing.v1.Controller

The daemon exposes the current set of blocked apps on its own interface. The
plugin discovers this state by two complementary mechanisms:

1. ActiveBlocks property — readable at any time. Returns all currently blocked
   apps with their block details (policy_id, reason, blocked_since,
   available_actions). The plugin reads this on startup, on reconnect, and
   periodically for reconciliation.
2. BlockStateChanged signal — emitted whenever a block is added or removed for
   an app. The plugin subscribes to this signal for low-latency state updates
   without polling.

Discovery flow:

Daemon blocks an app: EnforcerActor writes to ActiveBlocks state -> Daemon
updates ActiveBlocks property -> Daemon emits BlockStateChanged {app_id,
blocked: true} -> Plugin receives signal -> updates local overlay set -> If
app_id is currently focused -> overlay visible -> If app_id is not focused ->
overlay ready, shown on next focus

Daemon unblocks an app: EnforcerActor removes from ActiveBlocks state -> Daemon
updates ActiveBlocks property -> Daemon emits BlockStateChanged {app_id,
blocked: false} -> Plugin receives signal -> removes overlay from all windows of
that app_id

## Per-App Multi-Overlay Model

Blocking enforcement is keyed by app_id, never by window. The daemon is
window-count agnostic: it writes one entry per app_id to ActiveBlocks. Whether
the app has one window or fifty, the entry covers all windows.

The plugin treats every window of the app_id as a single logical surface. When
an app_id appears in ActiveBlocks, the plugin renders a block overlay over every
window owned by the app and traps both mouse and keyboard input on each blocked
window. The overlay presents the daemon-specified action buttons
(available_actions); a click on a button is reported back via UserAction.

Multiple distinct apps can be blocked at the same time. The plugin tracks an
unordered set of active overlays keyed by app_id, populated entirely from daemon
state (not from commands).

Overlay lifetime: an overlay persists until the daemon removes the app from
ActiveBlocks. Focus state does not affect overlay visibility — a blocked app's
overlay remains displayed even when another window is focused. This prevents
race conditions where a focus change causes the overlay to flicker or disappear.

### Focus handling

The plugin's focus-change handler reconciles overlay state against the daemon's
ActiveBlocks:

- User focuses app X: check if X is in local overlay set (which mirrors
  ActiveBlocks). If yes, the overlay is already rendered — nothing to do. If no,
  ensure no stale overlay for X.
- User focuses app Y (not blocked): no action needed. Overlays for other blocked
  apps remain visible.
- User focuses desktop (no window): all existing overlays remain visible.

The plugin never hides an overlay because focus moved away. Only a daemon
BlockStateChanged {blocked: false} or a user action that resolves the block
triggers overlay removal.

## Idle Detection

Idle/Resumed are produced by the compositor plugin, not logind. The plugin
tracks user activity (keyboard, mouse, touchpad, and video-player playback) and
exposes it via the ActivityChanged D-Bus signal on org.wellbeing.v1.Manager. The
daemon subscribes and maps Idle -> Idle (pause), Resumed -> Resumed (unpause)
PlatformEvents.

Key points:

- Idle/Resumed carry no app_id; the app they pause is the open interval from the
  most recent WindowFocused.
- Idle is the ONLY event that pauses an interval. Suspend/lock/logout/shutdown
  CLOSE it instead (see
  [03-linux-platform.md](./03-linux-platform.md#power--session-state-handling)).
- The plugin is responsible for idle debounce (e.g. a min-dwell before emitting
  Idle) so brief input gaps don't create noise segments.

## Plugin Registration (Reverse Discovery)

Each plugin instance connects to both system and session D-Bus busses
permanently at startup. The org.wellbeing.v1.Manager interface is registered on
both connections so the daemon can reach the plugin from either bus. The plugin
then runs resolveActiveDaemonBus() and calls Controller.RegisterPlugin() on the
resolved connection.

If the daemon is not reachable at startup, the plugin still holds both D-Bus
connections and installs its compositor hooks immediately. NameOwnerChanged
watchers on both connections provide event-driven notification when the daemon
appears — no polling needed.

The daemon identifies the plugin by the unique bus name (header.sender(), a
:1.xxx name assigned by the bus daemon) rather than a well-known name. Plugins
do not claim a well-known name — they connect anonymously (on both busses the
plugin holds an anonymous connection).

The daemon learns the caller's real identity from SO_PEERCRED
(kernel-authenticated uid).

Registration flow:

Daemon starts |-- Expose Controller.RegisterPlugin() |-- Expose ActiveBlocks
property |-- Expose BlockStateChanged signal

Plugin starts: |-- Create permanent connection to system bus (anonymous) |--
Create permanent connection to session bus (anonymous) |-- Register Manager
interface on both connections |-- Run resolveActiveDaemonBus() -> select system
or session |-- Call RegisterPlugin on the selected connection

Daemon receives RegisterPlugin: |-- Reads caller's unique bus name from
header.sender() |-- Reads SO_PEERCRED uid from connection credentials |--
Creates proxy to plugin via its unique bus name |-- Subscribes to FocusChanged +
ActivityChanged + UserAction streams |-- Plugin reads ActiveBlocks property
(initial state sync) |-- Plugin subscribes to BlockStateChanged signal (live
updates) |-- Plugin reconciles overlays: shows for any app in ActiveBlocks

Plugin disconnects (one connection drops): |-- If the dropped connection was the
active one: | |-- Daemon drops signal subscriptions for that unique bus name |
|-- Policy enforcement for that uid pauses | |-- Plugin re-resolves via the
other connection (if daemon still reachable) |-- If the dropped connection was
the inactive one: |-- No effect — active connection continues uninterrupted

Daemon restarts on different bus: |-- Plugin NameOwnerChanged fires on the stale
connection |-- resolveActiveDaemonBus() finds daemon on the other bus |--
Creates fresh daemon proxy, calls RegisterPlugin, syncs state

On disconnect, overlays on the compositor remain as-is (the plugin process
disappears with its compositor hooks). When the plugin reconnects, it reads
ActiveBlocks afresh and re-establishes all overlays.

## Multi-Instance Plugin Support

Each plugin instance reads the same ActiveBlocks property from the daemon. There
is no per-instance command routing. The daemon tracks each connected plugin by
its unique bus name, subscribes to its signals, and routes events into the
platform event stream. When a plugin disconnects, its subscriptions are dropped
and enforcement for that uid pauses until a new registration arrives.

Each plugin instance is responsible for showing overlays only for apps owned by
its user (the uid determined at registration via SO_PEERCRED). The daemon
includes the target uid in each ActiveBlocks entry, and the plugin filters
accordingly.

## Data Flow Summary

Daemon (EnforcerActor) | |-- Decides to block app X (policy evaluation) |-- Adds
X to ActiveBlocks (shared state) |-- Emits BlockStateChanged {app_id: X,
blocked: true} | v Plugin (via signal subscription + property read) | |--
Receives BlockStateChanged -> updates local overlay set |-- Reads ActiveBlocks
for full block details (reason, actions, etc.) |-- Renders overlay on all
windows of app X | |-- Focus changes to app X -> overlay already present |--
Focus changes to app Y -> overlay for X persists | |-- User clicks overlay
button -> Emits UserAction {app_id: X, action: Extra|Close} -> Daemon receives,
looks up policy_id from ActiveBlocks -> Grants extension or removes block ->
Updates ActiveBlocks -> plugin removes overlay

## Degraded Operation

If the daemon is not reachable at startup, the plugin still holds both D-Bus
connections and installs all compositor hooks immediately. NameOwnerChanged
watchers on both busses provide event-driven notification — no polling. When the
daemon appears on either bus, the plugin detects it, creates the daemon proxy,
and reads ActiveBlocks.

If the plugin disconnects (crashes), ActiveBlocks still exists on the daemon —
the daemon's state machine operates independently of plugin connectivity. When
the plugin restarts, it connects to both busses, resolves the daemon, reads
ActiveBlocks, and shows overlays for all currently blocked apps. No block state
is lost during a plugin restart.

If the daemon restarts on the same bus, the plugin's NameOwnerChanged watcher
fires, it re-registers and re-reads ActiveBlocks.

If the daemon restarts on a different bus (e.g., system daemon crashed and
session daemon started), the plugin's NameOwnerChanged watcher on the stale bus
fires, resolveActiveDaemonBus() detects the daemon on the other bus, and the
plugin creates a fresh daemon proxy on that connection. Recovery is transparent
— no polling, no plugin restart needed.
