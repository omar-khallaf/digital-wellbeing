# Architecture

This directory holds the design documentation, split into focused, hyperlinked
topics. Each concern lives in its own file (see the index below).

## Topics

| #   | Doc                                                      | Scope                                                                                                                      |
| --- | -------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------- |
| 01  | [01-rationale.md](./01-rationale.md)                     | "Why" essays: platform abstraction, gpui, D-Bus plugin IPC                                                                 |
| 02  | [02-platform.md](./02-platform.md)                       | The Platform trait, OverlayConfig, per-platform builders, concurrency model, PlatformEvent event model                     |
| 03  | [03-linux-platform.md](./03-linux-platform.md)           | Linux Platform impl: app metadata resolution, power/session state handling, compositor support                             |
| 04  | [04-plugin-ipc.md](./04-plugin-ipc.md)                   | org.wellbeing.v1.Manager D-Bus contract, declarative block state (ActiveBlocks), overlay lifecycle, multi-instance plugins |
| 05  | [05-daemon-auth.md](./05-daemon-auth.md)                 | Daemon-plugin trust model: D-Bus name ownership, SO_PEERCRED authentication, no crypto                                     |
| 06  | [06-daemon-dbus.md](./06-daemon-dbus.md)                 | org.wellbeing.v1.Controller D-Bus server, error mapping, GUI D-Bus client architecture                                     |
| 07  | [07-rbac.md](./07-rbac.md)                               | Per-user RBAC model, policy visibility, EnforcerActor per-user application, data-model changes                             |
| 08  | [08-modules.md](./08-modules.md)                         | Feature-per-directory layout, dependency flow, the blocking/overlay/ boundary, workspace tree                              |
| 09  | [09-state-flow.md](./09-state-flow.md)                   | Daemon-authoritative state, GUI cache architecture, runtime model, root/user UI, view models, daemon wiring                |
| 10  | [10-deployment.md](./10-deployment.md)                   | systemd unit, D-Bus policy files, install directory layout, D-Bus activation                                               |
| 11  | [11-implementation-plan.md](./11-implementation-plan.md) | Phased build plan (Phase A–F)                                                                                              |
| 12  | [12-open-questions.md](./12-open-questions.md)           | Open design questions and resolutions (resolved items kept)                                                                |
| 13  | [13-deployment-modes.md](./13-deployment-modes.md)       | System vs session daemon: bus/scope selection, GUI + plugin bus resolution, degraded mode, deploy artifacts                |

## Related Documentation

This directory is the system-design hub. Concern-specific docs live in sibling
directories under docs/ and link back here for shared context:

- docs/features/ — per-feature design: 01-blocking (overlay-only enforcement),
  02-categorization (DB-first categories + AI fallback), and 03-ui-design (gpui
  screens, component mapping, view models).
- docs/persistence/ — SQLite schema, migration policy, and the per-event / bulk
  write strategy.
- docs/quality/ — cross-cutting engineering: performance budget and testing
  philosophy.
- docs/planning/ — roadmap (planned features, non-goals).

The daemon is headless — no ui/ directories live in daemon feature trees. GUI
lives in the gui/ crate under gui/src/screens/<feature>/; blocking/ has no GUI
screen (overlay rendered by the compositor plugin, not gpui).

## Design Tenets

1. Device-Local — No cloud, no sync. This daemon tracks what happens on this
   machine only.
2. Platform-Agnostic Core — Domain model, tracking, and policy know nothing
   about the OS. They consume PlatformEvent and use overlay through the Platform
   trait.
3. Feature-Per-Directory — Each feature owns its domain, data access, core
   logic, and UI. Related code stays colocated.
4. Zero-Cost by Default — Type system enforces invariants at compile time.
   Traits monomorphize. Hot paths allocate zero.

## System Context

The system is split into two binaries communicating over D-Bus. The daemon
(wellbeing-daemon) owns all tracking, policy enforcement, and data. In system
mode (root) it is on the system bus and enforces all users; in session mode
(non-root) it is on the session bus and enforces only the user it runs as — see
13-deployment-modes.md. The GUI (wellbeing-gui) connects to the daemon
exclusively over D-Bus (resolving the correct bus) and has zero direct database
access. The compositor plugin uses the same bus as the daemon it registered
with.

Key architectural properties:

- Two binaries, one workspace — wellbeing-daemon (tokio, root, systemd service)
  and wellbeing-gui (gpui, user or root). Shared types in wellbeing-core crate.
  Separate dependency trees — no gpui in daemon, no diesel in GUI.
- Daemon owns SQLite — in system mode the database at
  /var/lib/digital-wellbeing/db.sqlite is mode 600, owned by root; in session
  mode it is at $XDG_DATA_HOME/digital-wellbeing/db.sqlite, mode 600,
  user-owned. The GUI never opens the database file — all data flows through the
  D-Bus API. WAL mode permits concurrent reads from daemon actors. See
  13-deployment-modes.md.
- D-Bus for everything — two well-known interfaces on the daemon's bus (system
  bus in system mode, session bus in session mode):
  - org.wellbeing.v1.Controller (daemon) — policy CRUD with RBAC, usage queries,
    state change signals
  - org.wellbeing.v1.Manager (plugin) — focus events, user actions, current
    focus property. Plugin reads block state from daemon's ActiveBlocks property
    (see 04-plugin-ipc.md)
- Per-user enforcement with RBAC — the daemon authorizes every D-Bus method call
  by the caller's uid (kernel-authenticated via SO_PEERCRED). In system mode,
  root (uid=0) can manage any user's policies; users manage only their own. In
  session mode the scope collapses to a single user (pass-through RBAC). See
  07-rbac.md and 13-deployment-modes.md.
- GUI as pure D-Bus client — no local SQLite, no in-process actors. The GUI
  subscribes to signals for cache-invalidation hints and re-queries data via
  method calls. A stale-while-revalidate cache prevents redundant queries on
  every render frame (see 09-state-flow.md#gui-cache-architecture).
- gpui + tokio in GUI — gpui's retained-mode UI runs on the main thread. A
  background tokio thread handles D-Bus connections, signal subscriptions, and
  method calls. Communication via mpsc channels.
- Plugin on the daemon's bus — the compositor plugin uses the same bus as the
  daemon it registered with (system bus in system mode, session bus in session
  mode); it resolves that bus the same way the GUI does. The daemon
  authenticates the plugin by SO_PEERCRED uid. The plugin reads block state from
  the daemon's ActiveBlocks D-Bus property and subscribes to BlockStateChanged
  for live updates. See 04-plugin-ipc.md and
  13-deployment-modes.md#plugin-resolution.
- Overlay-only enforcement — blocks operate by showing an overlay that traps
  input. No process signal operations.

### Why the daemon–GUI split

Why:

- RBAC — in system mode root runs the daemon and users run the GUI; policy CRUD
  is authorized by D-Bus caller credentials (uid). In session mode the daemon
  runs as the user and enforces only that user. See 13-deployment-modes.md.
- Multi-user — in system mode one daemon serves all users on the machine; each
  user sees their own usage data and policies, subject to access control. In
  session mode a single-user daemon enforces only its own user.
- Separation of concerns — the daemon owns tracking, enforcement, and data; the
  GUI is a pure client. No gpui dependency in the daemon, no SQLite dependency
  in the GUI.
- Security — the daemon owns the SQLite database (mode 600, root-owned in system
  mode, user-owned in session mode). The GUI has zero direct database access —
  all data flows through the D-Bus API.

Constraints (from AGENTS.md and the design docs):

- Device-local only (no cloud, no sync)
- Overlay-only enforcement (no process signals)
- SQLite as source of truth
- Plugin IPC via D-Bus (single interface contract)
- Per-feature directory layout (domain / data / core / ui)
- Newtype boundary gate for all raw strings
- Clock trait for deterministic testing

### Event Processing Pipeline

All focus events are processed through a gate-first pipeline — the EnforcerActor
evaluates policy before any event is persisted. The enforcer runs one evaluation
cycle per FocusChanged event.

WindowFocused arrives from the plugin as a PlatformEvent. The EnforcerActor acts
as gatekeeper:

1. It queries the app's daily_usage plus active policies.
2. It evaluates the app against policies and usage before any DB write. If the
   verdict is Block, the previous app's open interval is closed with an
   Unfocused event, the app is added to ActiveBlocks so the plugin renders an
   overlay, no WindowFocused is written for the new app, and the blocked app
   never enters the event log. If the verdict is Notify, the previous interval
   is closed, a WindowFocused is written for the new app, the app proceeds
   normally, and a desktop notification is sent. If the policy has a repeat
   interval, a real-time timer fires at that interval to re-notify the user
   while the app remains focused. A limit timer is also started for other
   policies. If the verdict is Ok, the previous interval is closed, a
   WindowFocused is written for the new app, and a limit timer is started for
   TimeLimit policies. When that timer fires, the app is re-evaluated and if the
   limit is exceeded the overlay is shown immediately without waiting for the
   next focus switch.

Notify path: when a Notify policy triggers, the app proceeds normally — events
are written and usage accumulates. The EnforcerActor sends a desktop
notification via the platform notify method (freedesktop.org D-Bus Notifications
spec). If the policy has a notification_repeat_interval_minutes, a real-time
timer fires at that interval to re-notify the user while the app remains
focused.

Limit timer re-triggering: when an app passes policy check and focus is granted,
the EnforcerActor calculates the remaining seconds until its limit is reached. A
tokio sleep task is spawned for that duration. When it fires, the EnforcerActor
re-evaluates the app's policy. If the app is still focused and its accumulated
usage now exceeds the limit, the overlay is shown immediately — without waiting
for the next focus switch.

Key consequences:

- Blocked apps never appear in the event log — only tracked focus is recorded.
- The Unfocused written during a block closes the previous app's interval (A),
  not the blocked app's (which was never opened).
- Timer enforcement catches limit expiry during continuous use of a single app,
  not just on focus switches.
- Notify policies do NOT block — the app's focus interval proceeds normally, and
  notifications are advisory.
