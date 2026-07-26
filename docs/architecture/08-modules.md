# Feature / Core Module Design

Each feature is a self-contained directory that mirrors the application's layers
internally. The directory structure communicates the domain, not the tech stack
— this is the screaming architecture pattern.

## Directory Layout

Each feature directory contains three layers:

- domain — Domain types, state machines, pure business rules
- data — Persistence (SQLite via diesel), query builders
- core — Actor, business logic, wires domain + data + platform

UI for any feature lives in the gui crate (gui/src/dashboard/,
gui/src/policies/, gui/src/reports/), not inside the daemon's feature
directories. The daemon is headless — it contains no gpui and no UI code.

## Dependency Flow

Rules:

- domain/ modules have zero dependencies on tokio, diesel, gpui, or any
  infrastructure.
- data/ modules depend on domain/ and store/ only.
- core/ modules are the actors — they wire domain + data + platform.
- The daemon is headless: feature directories (policy/, categorization/,
  blocking/, reports/) contain only domain/, data/, core/ — no UI code.
- UI lives in the gui/ crate, organized per feature under gui/src/dashboard/,
  gui/src/policies/, and gui/src/reports/. Screens read derived data via the
  D-Bus client + in-memory cache — never SQLite directly — and subscribe to
  daemon signals for cache invalidation. blocking/ has no GUI screen: it is
  overlay-only enforcement, and the overlay is rendered by the compositor
  plugin, not gpui.
- No circular dependencies between features.

## The blocking/ Boundary

The blocking feature is self-contained in blocking/domain/, blocking/data/, and
blocking/core/. It does not introduce a second Platform trait — it uses the
top-level Platform trait for notification and event ingestion. The blocking
feature owns:

- Domain types specific to blocking: BlockedAppEntry, BlockReason — these are
  blocking/domain/ types used by the daemon's D-Bus interface and policy
  evaluation.
- The event buffer: EventBuffer accumulates PlatformEvents for batch
  persistence.
- The enforcement actor: EnforcerActor<P, C> receives PlatformEvents, buffers
  them, and evaluates policies from the database at minute-tick boundaries.
- The disconnect handler logic: log on plugin disconnect, re-show overlay on
  reconnect.

The top-level platform/ trait defines how to perform an operation (D-Bus method
call, signal dispatch). blocking/ defines when to perform it (event buffering,
policy evaluation, block state updates).

Dependency direction:

- blocking/core -> blocking/data (persistence)
- blocking/core -> top-level platform (calls notify)
- blocking/core -> policy (evaluates policies)

This separation exists because enforcement logic is purely a blocking feature
concern. Other features (tracking, categorization, reports) never need to know
about event buffering or block state machines.

## Workspace Layout

The workspace is organized into crates/ with three main crates plus plugins:

- core/ — Shared library with zero dependencies on tokio, diesel, or gpui
- daemon/ — Tokio daemon binary, owns SQLite, actors, and D-Bus server
- gui/ — gpui GUI binary, pure D-Bus client with in-memory cache

Within daemon/src/ each feature owns domain/, data/, and core/. The gui/ crate
mirrors this with dashboard/, policies/, and reports/ per feature. Plugins live
under plugins/ (Hyprland C++ plugin in v1). Migration files are shared at
migrations/ and deployed artifacts live under deploy/ (D-Bus policy, systemd
units, service activation files).

## Dependency Edges

core -> serde, chrono, thiserror, zvariant (no tokio/diesel/gpui) daemon ->
core + tokio + zbus + diesel/diesel-async + nix + procfs gui -> core + gpui +
zbus + tokio (sync, rt, macros)

The GUI crate explicitly does not depend on diesel, diesel-async, or any
database library. It accesses all data through D-Bus method calls.
