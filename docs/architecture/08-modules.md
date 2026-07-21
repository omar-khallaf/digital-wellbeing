# Feature / Core Module Design

Each feature is a self-contained directory that mirrors the application's layers
internally:

```
tracking/                    # Feature: usage tracking (daemon side)
├── domain/                  # Domain types, state machine
│   └── mod.rs
├── data/                    # Persistence (SQLite via diesel)
│   └── mod.rs
└── core/                    # Actor, business logic
    └── mod.rs
# UI for this feature lives in the gui crate (gui/src/screens/...), NOT here.
# The daemon is headless — it contains no gpui and no UI code.
```

This is the **screaming architecture** pattern: the directory structure
communicates the domain, not the tech stack.

## Dependency Flow

```
core/ → zero project deps
    │
    ▼
platform/ → core/
    │
    ▼
tracking/core ──→ tracking/domain, tracking/data, platform/
policy/core   ──→ policy/domain, policy/data, platform/
blocking/core ──→ blocking/domain, blocking/overlay, platform/
    │
    ▼
categorization ──→ core/, store/ (DB queries + domain types)
    │
    ▼
gui/ (separate binary) ──→ core/ (shared types) + D-Bus client
    │   screens/dashboard, screens/policy, screens/reports
    │   → subscribes to daemon D-Bus signals (BlockStateChanged,
    │     DailyUsageChanged, PolicyMutated) for cache invalidation
    │
    ▼
daemon main.rs (wires actors) · gui/main.rs (gpui + bg tokio)
```

Rules:

- `domain/` modules have zero dependencies on tokio, diesel, gpui, or any
  infrastructure.
- `data/` modules depend on `domain/` and `store/` only.
- `core/` modules are the actors — they wire domain + data + platform.
- The **daemon is headless**: feature directories (`tracking/`, `policy/`,
  `categorization/`, `blocking/`, `reports/`) contain only `domain/`, `data/`,
  `core/` — and `overlay/` for `blocking/`. They hold no gpui and no UI code.
- **UI lives in the `gui/` crate**, organized per feature under
  `gui/src/screens/<feature>/` (Dashboard, Policies, Reports). Screens read
  derived data via the D-Bus client + in-memory cache — never SQLite directly —
  and subscribe to daemon signals for cache invalidation. `blocking/` has no GUI
  screen: it is overlay-only enforcement, and the overlay is rendered by the
  compositor plugin, not gpui.
- No circular dependencies between features.

## The `blocking/overlay/` Boundary

Renamed from `blocking/platform/` to avoid confusion with the top-level
`platform/` trait. `blocking/overlay/` is **not** a second Platform trait — it
is the blocking feature's extension of the Platform contract. It holds:

- **Domain types specific to blocking**: `OverlayConfig`, `OverlayAction`,
  `BlockOverlayState`, `BlockReason` — these are `blocking/domain/` types that
  the Platform trait references.
- **The overlay lifecycle state machine**: `BlockingState` (Idle → OverlayShown
  → AwaitingChoice → PluginLost → ...).
- **The disconnect handler logic**: log on plugin disconnect, re-show overlay on
  reconnect.
- **The overlay action router**: translates user choices (Extra, Close) into the
  appropriate DB writes.

The top-level `platform/` trait defines _how_ to perform an operation (D-Bus
method call, signal dispatch). `blocking/overlay/` defines _when_ to perform it
(lifecycle, state transitions, optionality).

Dependency direction:

- `blocking/core` → `blocking/overlay` (reads state, drives transitions)
- `blocking/core` → top-level `platform` (calls show/hide overlay)

This separation exists because the overlay lifecycle is purely a blocking
feature concern. Other features (tracking, policy, reports) never need to know
about disconnect handling or overlay state machines.

## Workspace Layout

```
digital-wellbeing/
├── Cargo.toml                     # [workspace] root
├── crates/
│   ├── core/                      # Shared lib (no tokio, no diesel, no gpui)
│   │   └── Cargo.toml             #   deps: serde, chrono, thiserror, zvariant
│   │
│   ├── daemon/                    # Tokio daemon binary
│   │   ├── Cargo.toml             #   deps: core, tokio, zbus, diesel-async+sqlite
│   │   └── src/
│   │       ├── main.rs            #   Entrypoint, wiring
│   │       ├── lib.rs             #   Re-exports for integration testing
│   │       ├── store/
│   │       │   ├── mod.rs
│   │       │   ├── connection.rs  #   DbPool / StoreBuilder
│   │       │   ├── migrations.rs  #   embed_migrations!
│   │       │   └── schema.rs      #   Diesel table! definitions
│   │       ├── platform/
│   │       │   ├── mod.rs         #   Platform trait, PlatformEvent, OverlayConfig
│   │       │   └── linux.rs       #   LinuxPlatform, ManagerClient (system D-Bus)
│   │       ├── dbus/
│   │       │   └── mod.rs         #   org.wellbeing.v1.Controller server + RBAC
│   │       ├── tracking/          #   domain, data, core
│   │       ├── policy/            #   domain, data, core, engine.rs
│   │       ├── categorization/    #   data, core
│   │       ├── blocking/          #   domain, overlay, core
│   │       └── reports/           #   domain, data, core
│   │
│   └── gui/                       # gpui GUI binary
│       ├── Cargo.toml             #   deps: core, gpui, zbus, tokio (minimal)
│       └── src/
│           ├── main.rs            #   gpui::Application::run + bg tokio thread
│           ├── app.rs             #   App shell (TitleBar, TabBar, tray, user mode)
│           ├── dbus/
│           │   ├── mod.rs         #   DaemonClient (zbus proxy + signal coalescing)
│           │   └── signal.rs      #   SignalCoalescer
│           ├── cache/
│           │   └── mod.rs         #   ClientCache<K,V> stale-while-revalidate
│           ├── dashboard/
│           │   ├── mod.rs         #   Screen registration
│           │   └── view.rs        #   DashboardViewModel + gpui component tree
│           ├── policies/
│           │   ├── mod.rs         #   Screen registration
│           │   └── view.rs        #   PoliciesViewModel + gpui components
│           └── reports/
│               ├── mod.rs         #   Screen registration
│               └── view.rs        #   EventLog, ExportDialog
│
├── plugins/
│   └── hyprland/                  # C++ plugin
│       └── CMakeLists.txt
│
├── migrations/                    # Shared migration files
│
├── docs/
│   ├── architecture/            # Focused system-design docs
│   ├── features/                # Per-feature design: 01-blocking, 02-categorization, 03-ui-design
│   ├── persistence/             # 01-database (schema, migrations, write strategy)
│   ├── quality/                 # 01-performance, 02-testing
│   └── planning/                # 01-roadmap
│
├── deploy/
│   ├── digital-wellbeing-daemon.service  # systemd unit
│   ├── org.wellbeing.v1.Controller.conf       # D-Bus system policy
│   └── org.wellbeing.v1.Manager.conf      # D-Bus system policy
│
└── README.md
```

## Dependency Edges

```
core        → serde, chrono, thiserror, zvariant  (no tokio/diesel/gpui)
daemon      → core + tokio + zbus + diesel/diesel-async + nix + procfs
gui         → core + gpui + zbus + tokio (sync, rt, macros)
```

The GUI crate explicitly does **not** depend on diesel, diesel-async, or any
database library. It accesses all data through D-Bus method calls.
