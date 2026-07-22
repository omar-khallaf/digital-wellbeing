# Feature / Core Module Design

Each feature is a self-contained directory that mirrors the application's layers
internally. The directory structure communicates the domain, not the tech stack
— this is the screaming architecture pattern.

## Directory Layout

Each feature directory contains three layers:

- domain — Domain types, state machines, pure business rules
- data — Persistence (SQLite via diesel), query builders
- core — Actor, business logic, wires domain + data + platform

UI for any feature lives in the gui crate (gui/src/screens/...), not inside the
daemon's feature directories. The daemon is headless — it contains no gpui and
no UI code.

## Dependency Flow

core/ -> zero project dependencies | v platform/ -> core/ | v tracking/core ->
tracking/domain, tracking/data, platform/ policy/core -> policy/domain,
policy/data, platform/ blocking/core -> blocking/domain, blocking/overlay,
platform/ | v categorization -> core/, store/ (DB queries + domain types) | v
gui/ (separate binary) -> core/ (shared types) + D-Bus client |
screens/dashboard, screens/policy, screens/reports | subscribes to daemon D-Bus
signals (BlockStateChanged, | DailyUsageChanged, PolicyMutated) for cache
invalidation | v daemon main.rs (wires actors) · gui/main.rs (gpui + bg tokio)

Rules:

- domain/ modules have zero dependencies on tokio, diesel, gpui, or any
  infrastructure.
- data/ modules depend on domain/ and store/ only.
- core/ modules are the actors — they wire domain + data + platform.
- The daemon is headless: feature directories (tracking/, policy/,
  categorization/, blocking/, reports/) contain only domain/, data/, core/ — and
  overlay/ for blocking/. They hold no gpui and no UI code.
- UI lives in the gui/ crate, organized per feature under
  gui/src/screens/<feature>/ (Dashboard, Policies, Reports). Screens read
  derived data via the D-Bus client + in-memory cache — never SQLite directly —
  and subscribe to daemon signals for cache invalidation. blocking/ has no GUI
  screen: it is overlay-only enforcement, and the overlay is rendered by the
  compositor plugin, not gpui.
- No circular dependencies between features.

## The blocking/overlay/ Boundary

Renamed from blocking/platform/ to avoid confusion with the top-level platform/
trait. blocking/overlay/ is not a second Platform trait — it is the blocking
feature's extension of the Platform contract. It holds:

- Domain types specific to blocking: OverlayConfig, OverlayAction,
  BlockOverlayState, BlockReason — these are blocking/domain/ types that the
  Platform trait references.
- The overlay lifecycle state machine: BlockingState (Idle -> OverlayShown ->
  AwaitingChoice -> PluginLost -> ...).
- The disconnect handler logic: log on plugin disconnect, re-show overlay on
  reconnect.
- The overlay action router: translates user choices (Extra, Close) into the
  appropriate DB writes.

The top-level platform/ trait defines how to perform an operation (D-Bus method
call, signal dispatch). blocking/overlay/ defines when to perform it (lifecycle,
state transitions, optionality).

Dependency direction:

- blocking/core -> blocking/overlay (reads state, drives transitions)
- blocking/core -> top-level platform (calls show/hide overlay)

This separation exists because the overlay lifecycle is purely a blocking
feature concern. Other features (tracking, policy, reports) never need to know
about disconnect handling or overlay state machines.

## Workspace Layout

The workspace is organized into crates/ with three main crates plus plugins:

- core/ — Shared library with zero dependencies on tokio, diesel, or gpui
- daemon/ — Tokio daemon binary, owns SQLite, actors, and D-Bus server
- gui/ — gpui GUI binary, pure D-Bus client with in-memory cache

Within daemon/src/ each feature owns domain/, data/, core/, and overlay/ for
blocking. The gui/ crate mirrors this with screens/ per feature. Plugins live
under plugins/ (Hyprland C++ plugin in v1). Migration files are shared at
migrations/ and deployed artifacts live under deploy/ (D-Bus policy, systemd
units, service activation files).

## Dependency Edges

core -> serde, chrono, thiserror, zvariant (no tokio/diesel/gpui) daemon ->
core + tokio + zbus + diesel/diesel-async + nix + procfs gui -> core + gpui +
zbus + tokio (sync, rt, macros)

The GUI crate explicitly does not depend on diesel, diesel-async, or any
database library. It accesses all data through D-Bus method calls.
