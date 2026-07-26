# Architecture

This directory holds the design documentation, split into focused, hyperlinked
topics. Each concern lives in its own file (see the index below).

## Topics

| #   | Doc                                                      | Scope                                                                                                                     |
| --- | -------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------- |
| 01  | [01-rationale.md](./01-rationale.md)                     | "Why" essays: platform abstraction, gpui, D-Bus plugin IPC                                                                |
| 02  | [02-platform.md](./02-platform.md)                       | The Platform trait, OverlayConfig, per-platform builders, concurrency model, PlatformEvent event model                    |
| 03  | [03-linux-platform.md](./03-linux-platform.md)           | Linux Platform impl: app metadata resolution, power/session state handling, compositor support                            |
| 04  | [04-plugin-ipc.md](./04-plugin-ipc.md)                   | org.wellbeing.v1.Manager D-Bus contract, declarative block state (BlockedApps), overlay lifecycle, multi-instance plugins |
| 05  | [05-daemon-auth.md](./05-daemon-auth.md)                 | Daemon-plugin trust model: D-Bus name ownership, SO_PEERCRED authentication, no crypto                                    |
| 06  | [06-daemon-dbus.md](./06-daemon-dbus.md)                 | org.wellbeing.v1.Controller D-Bus server, error mapping, GUI D-Bus client architecture                                    |
| 07  | [07-rbac.md](./07-rbac.md)                               | Per-user RBAC model, policy visibility, EnforcerActor per-user application, data-model changes                            |
| 08  | [08-modules.md](./08-modules.md)                         | Feature-per-directory layout, dependency flow, daemon/gui boundary, workspace tree                                        |
| 09  | [09-state-flow.md](./09-state-flow.md)                   | Daemon-authoritative state, GUI cache architecture, runtime model, root/user UI, view models, daemon wiring               |
| 10  | [10-deployment.md](./10-deployment.md)                   | systemd unit, D-Bus policy files, install directory layout, D-Bus activation                                              |
| 11  | [11-implementation-plan.md](./11-implementation-plan.md) | Phased build plan (Phase A–F)                                                                                             |
| 12  | [12-open-questions.md](./12-open-questions.md)           | Open design questions and resolutions (resolved items kept)                                                               |
| 13  | [13-deployment-modes.md](./13-deployment-modes.md)       | System vs session daemon: bus/scope selection, GUI + plugin bus resolution, degraded mode, deploy artifacts               |

## Related Documentation

This directory is the system-design hub. Concern-specific docs live in sibling
directories under docs/ and link back here for shared context:

- docs/features/ — per-feature design: 01-blocking (overlay-only enforcement),
  02-categorization (DB-first categories + AI fallback), and 03-ui-design (gpui
  screens, component mapping, view models).
- docs/persistence/ — SQLite schema, migration policy, and the buffered flush
  write strategy.
- docs/quality/ — cross-cutting engineering: performance budget and testing
  philosophy.
- docs/planning/ — roadmap (planned features, non-goals).

The daemon is headless — no ui/ directories live in daemon feature trees. GUI
lives in the gui/ crate under gui/src/dashboard/, gui/src/policies/, and
gui/src/reports/; blocking/ has no GUI screen (overlay rendered by the
compositor plugin, not gpui).

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
    focus property. Plugin reads block state from daemon's BlockedApps property
    (see 04-plugin-ipc.md)
- Per-user enforcement with RBAC — the daemon authorizes every D-Bus method call
  by the caller's uid (kernel-authenticated via SO_PEERCRED). In system mode,
  root (uid=0) can manage any user's policies; users manage only their own. In
  session mode the scope collapses to a single user (pass-through RBAC). See
  07-rbac.md and 13-deployment-modes.md.
- GUI as pure D-Bus client — no local SQLite, no in-process actors. The GUI
  subscribes to signals for cache-invalidation hints and re-queries data via
  method calls. An explicit-invalidation cache prevents redundant queries on
  every render frame (see 09-state-flow.md#gui-cache-architecture).
- gpui + tokio in GUI — gpui's retained-mode UI runs on the main thread. A
  background tokio thread handles D-Bus connections, signal subscriptions, and
  method calls. Communication via mpsc channels.
- Plugin on the daemon's bus — the compositor plugin uses the same bus as the
  daemon it registered with (system bus in system mode, session bus in session
  mode); it resolves that bus the same way the GUI does. The daemon
  authenticates the plugin by SO_PEERCRED uid. The plugin reads block state from
  the daemon's BlockedApps D-Bus property and subscribes to BlockedAppsChanged
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
- Per-feature directory layout (domain / data / core)
- Newtype boundary gate for all raw strings
- Clock trait for deterministic testing

### Event Processing — True Event Log

The event log is an honest append-only record. Every event is written — no
synthetic Unfocus events, no dropped Focus events.

**Event arrives from plugin:**

1. `INSERT INTO apps (app_class) VALUES (?) ON CONFLICT DO NOTHING` — upsert.
2. Plugin sends `Event` signal with tag:
   - `tag=0` (Focus) if app is NOT in `BlockedApps`.
   - `tag=2` (Block) if app IS in `BlockedApps`.
3. Daemon writes the event as-is: `event_type=0` for Focus, `event_type=8` for
   Blocked. No synthetic events, no gate. A Focus event naturally terminates the
   previous interval; a Blocked event terminates without accumulating time.
4. If the event was Focus: query usage + policies → `evaluate()`.

**If Block or TimeLimit exceeded (evaluate result):**

- Update `BlockedApps` D-Bus property → emit `BlockedAppsChanged`.
- Plugin receives `BlockedAppsChanged`, checks currently focused window — if
  it's now in `BlockedApps`, plugin **immediately** sends `Event(tag=2=Block)`
  without waiting for a focus switch.
- On next focus for this app, plugin sends `Event(tag=2=Block)` directly.
- The initial Focus→Blocked span IS accumulated (ms).

**If Notify:**

- Send one-shot `platform.notify()`. Interval continues normally.

**If Allow or unrestricted:**

- No further action. Interval continues normally.

**Key consequences:**

- Plugin startup: reads `BlockedApps` property, shows overlay for any blocked
  apps that are currently focused, sends `Event(tag=2=Block)` if needed.
- On `BlockedAppsChanged`: plugin checks currently focused window — if now
  blocked, immediately sends `Event(tag=2=Block)` (no focus switch needed).
- No synthetic events. Focus is always written, Blocked explicitly terminates.
- Focus→Blocked span counts as tracked time (ms). Blocked→next-Focus does not —
  Blocked only terminates, never starts an interval.
- Notify is one-shot — no repeat timer.
- No per-app `tokio::sleep` timers: a lightweight per-minute tick re-evaluates
  the single currently focused app, catching TimeLimit expiry during continuous
  use.
