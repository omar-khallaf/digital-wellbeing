# Digital Wellbeing

A digital wellbeing system for Wayland compositors. Tracks app usage, enforces
time limits, and helps you maintain focus — no cloud, no surveillance.

Inspired by Android's Digital Wellbeing, built for Linux desktop.

## Project Status

**Design / Specification Phase — partially scaffolded.** The full architecture,
D-Bus contracts, and module layout are specified; implementation has not started
on most components.

## Architecture

The system is split into **two binaries communicating over D-Bus**, with an
optional compositor plugin for overlay enforcement:

- **`wellbeing-daemon`** — tokio async daemon (runs as root in **system mode**
  or non-root in **session mode**; mode selected at startup by uid) that owns
  all tracking, policy enforcement, and SQLite data. In system mode it claims
  `org.wellbeing.v1.Controller` on the **system bus** and enforces per-user
  RBAC; in session mode it claims the name on the **session bus** and enforces a
  single user. Exposes policy CRUD and usage queries over D-Bus.
- **`wellbeing-gui`** — gpui desktop app that connects exclusively over D-Bus to
  the daemon. Never opens SQLite directly. Uses an in-memory
  stale-while-revalidate cache. **Resolves the daemon's bus at runtime** via a
  4-step algorithm (system present → session present → activate system →
  activate session), never hardcodes a bus.
- **Compositor plugin** (`org.wellbeing.v1.Manager`) — renders block overlays
  via OpenGL and emits `FocusChanged` / `ActivityChanged` (with
  `FocusActivityTag` enum) signals. Runs in the user's compositor session and
  resolves the daemon's bus using the **identical 4-step algorithm** as the GUI,
  so it always lands on the same daemon instance. This guarantees exactly one
  enforcing daemon per user — no double overlay.

```text
                     ┌──────────────────────────────────────────┐
                     │  wellbeing-daemon (system/session mode)  │
                     │                                          │
                     │  ┌───────────tokio runtime─────────────┐ │
                     │  │ TrackerActor  PolicyEngine          │ │
                     │  │ EnforcerActor ReportBuilder         │ │
                     │  │                                     │ │
                     │  │  ──write──→ SQLite ───┐             │ │
                     │  └────────────┬────────────────────────┘ │
                     │               │ D-Bus signals            │
                     │               │ (BlockStateChanged,      │
                     │               │  DailyUsageChanged,      │
                     │               │  PolicyMutated)          │
                     │               ▼                          │
                     │  ┌────────────────────────────────────┐  │
                     │  │  D-Bus server                      │  │
                     │  │  org.wellbeing.v1.Controller       │  │
                     │  │  system bus (root) /               │  │
                     │  │  session bus (non-root)            │  │
                     │  │  Methods: ListPolicies,            │  │
                     │  │   CreatePolicy, GetDailyUsage,     │  │
                     │  │   GetUsageRange, ListCategories,   │  │
                     │  │   SetAppCategory                   │  │
                     │  └────────────────────────────────────┘  │
                     │                                          │
                     │  ┌────────────────────────────────────┐  │
                     │  │  PluginRegistry (per-instance,     │  │
                     │  │  daemon's resolved bus)            │  │
                     │  │  → plugin Overlay(v)               │  │
                     │  └────────────────────────────────────┘  │
                     └────────────────────┬─────────────────────┘
                                          │
                            ┌─────────────┴─────────────┐
                            │                           │
                            │ D-Bus (daemon's bus)      │ D-Bus (daemon's bus)
                            │                           │
                            ▼                           ▼
               ┌────────────────────────┐    ┌────────────────────────┐
               │  wellbeing-gui         │    │  Compositor plugin     │
               │  (user or root)        │    │  (user session)        │
               │                        │    │                        │
               │  ┌──────────────────┐  │    │  org.wellbeing.v1.     │
               │  │ gpui (main thr.) │  │    │  Manager               │
               │  │  render loop     │  │    │                        │
               │  └────────┬─────────┘  │    │ Overlay(v)             │
               │           │ mpsc       │    │  FocusChanged [signal] │
               │  ┌────────┴─────────┐  │    │  CurrentFocus [prop]   │
               │  │ tokio (bg thr.)  │  │    └────────────────────────┘
               │  │  D-Bus client    │  │
               │  │  zbus stubs +    │  │
               │  │  signal sub      │  │
               │  └──────────────────┘  │
               └────────────────────────┘
```

### Architecture docs

Full design reasoning, D-Bus contracts, and component specs in
[architecture/README.md](docs/architecture/README.md):

| Doc                                                                      | Scope                                                 |
| ------------------------------------------------------------------------ | ----------------------------------------------------- |
| [01-rationale.md](docs/architecture/01-rationale.md)                     | "Why" essays — platform abstraction, gpui, D-Bus IPC  |
| [02-platform.md](docs/architecture/02-platform.md)                       | `Platform` trait, `OverlayConfig`, event model        |
| [03-linux-platform.md](docs/architecture/03-linux-platform.md)           | Linux platform impl: metadata, power state            |
| [04-plugin-ipc.md](docs/architecture/04-plugin-ipc.md)                   | Plugin D-Bus contract, signed overlay tokens          |
| [05-daemon-auth.md](docs/architecture/05-daemon-auth.md)                 | Ed25519 signing, `DaemonPublicKey`, replay handling   |
| [06-daemon-dbus.md](docs/architecture/06-daemon-dbus.md)                 | `org.wellbeing.v1.Controller` server, error mapping   |
| [07-rbac.md](docs/architecture/07-rbac.md)                               | Per-user RBAC, policy visibility per uid              |
| [08-modules.md](docs/architecture/08-modules.md)                         | Feature-per-directory layout, dependency flow         |
| [09-state-flow.md](docs/architecture/09-state-flow.md)                   | Daemon-authoritative state, GUI cache architecture    |
| [10-deployment.md](docs/architecture/10-deployment.md)                   | systemd unit, D-Bus policy files, install layout      |
| [11-implementation-plan.md](docs/architecture/11-implementation-plan.md) | Phased build plan (Phase A–F)                         |
| [12-open-questions.md](docs/architecture/12-open-questions.md)           | Open design questions and resolutions                 |
| [13-deployment-modes.md](docs/architecture/13-deployment-modes.md)       | System vs session daemon modes, 4-step bus resolution |

## Workspace Layout

```
crates/
├── core/src/           # well-being-core: valuetypes, errors, clock, domain (shared)
├── daemon/src/
│   ├── main.rs         # Actor wiring, D-Bus server start
│   ├── lib.rs          # Re-exports for integration tests
│   ├── store/          # DbPool, migrations, schema
│   ├── platform/       # Platform trait + LinuxPlatform + ManagerClient
│   ├── dbus/           # org.wellbeing.v1.Controller server + RBAC
│   ├── tracking/       # domain/ data/ core/ (TrackerActor)
│   ├── policy/         # domain/ data/ core/ (PolicyEngine)
│   ├── categorization/ # data/ core/ (Categorizer + AI fallback)
│   ├── blocking/       # domain/ overlay/ core/ (EnforcerActor)
│   └── reports/        # domain/ data/ core/ (aggregate queries)
└── gui/
    └── src/
        ├── main.rs            #   gpui::Application::run + bg tokio thread
        ├── app.rs             #   App shell (TitleBar, TabBar, tray, user mode)
        ├── dbus/              #   DaemonClient (zbus proxy + signal coalescing)
        ├── cache/             #   ClientCache<K,V> stale-while-revalidate
        ├── dashboard/
        │   ├── mod.rs         #   Screen registration
        │   └── view.rs        #   DashboardViewModel + gpui component tree
        ├── policies/
        │   ├── mod.rs         #   Screen registration
        │   └── view.rs        #   PoliciesViewModel + gpui components
        └── reports/
            ├── mod.rs         #   Screen registration
            └── view.rs        #   EventLog, ExportDialog
```

**Dependency rules:** `core/` → zero deps → feature `*/domain` → `*/data` →
`*/core`. Daemon is headless — no `ui/` dirs in daemon features. GUI accesses
all data via D-Bus, never SQLite directly. No cycles.

## Building

```bash
# Build the Rust workspace
cargo build

# Release build
cargo build --release

# Build a specific crate
cargo build -p wellbeing-daemon
cargo build -p wellbeing-gui

# Build the Hyprland compositor plugin (wellbeing-lockdown.so)
cd plugins/hyprland && cmake --preset linux-host && cmake --build --preset release-host

# Run tests
cargo test

# Lint
cargo clippy -- -D warnings
```

## Commands

| Command                                                                                   | Description                 |
| ----------------------------------------------------------------------------------------- | --------------------------- |
| `cargo build --release`                                                                   | Release build               |
| `cargo test`                                                                              | Run all tests               |
| `cargo clippy -- -D warnings`                                                             | Lint check                  |
| `cargo fmt --check`                                                                       | Format check                |
| `cargo build -p wellbeing-daemon`                                                         | Build daemon only           |
| `cargo build -p wellbeing-gui`                                                            | Build GUI only              |
| `cd plugins/hyprland && cmake --preset linux-host && cmake --build --preset release-host` | Build Hyprland plugin (.so) |

## Design Decisions

| Decision                                      | Rationale                                                                                                            | See                                                        |
| --------------------------------------------- | -------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------- |
| Two binaries (daemon + GUI)                   | Root daemon owns SQLite; GUI runs unprivileged; no direct DB access in GUI; separate dep trees                       | [architecture/README.md](docs/architecture/README.md)      |
| D-Bus for everything                          | Daemon, GUI, and plugin share a single IPC contract; bus daemon handles auth via SO_PEERCRED                         | [architecture/README.md](docs/architecture/README.md)      |
| Plugin resolves daemon's bus (4-step)         | Plugin runs the same system→session→activate→activate algorithm as GUI to find the daemon on whatever bus it owns    | [04-plugin-ipc.md](docs/architecture/04-plugin-ipc.md)     |
| Per-user RBAC                                 | Daemon authenticates every D-Bus call by caller uid; root manages any user; users manage only their own              | [07-rbac.md](docs/architecture/07-rbac.md)                 |
| Platform abstraction                          | OS-specific code behind `Platform` trait; Linux first, Windows later                                                 | [02-platform.md](docs/architecture/02-platform.md)         |
| Overlay-only enforcement                      | No process signals — compositor plugin traps input via overlay; no CAP_SYS_PTRACE, no crash recovery races           | [01-blocking.md](docs/features/01-blocking.md)             |
| Categories as extra feature                   | Core tracks apps; categories are a derived grouping                                                                  | [02-categorization.md](docs/features/02-categorization.md) |
| ViewModel layer separates data from rendering | Each feature `ui/` constructs ViewModels from cache + signals; gpui renders ViewModels, not D-Bus actors             | [09-state-flow.md](docs/architecture/09-state-flow.md)     |
| Clock trait for deterministic time            | SystemClock prod / VirtualClock test; injected into all time-dependent actors                                        | [02-testing.md](docs/quality/02-testing.md)                |
| gpui-component library                        | Pre-built components (TabBar, Chart, Select, Settings, Input, Switch) for UI; avoids custom gpui layout              | [03-ui-design.md](docs/features/03-ui-design.md)           |
| Stale-while-revalidate GUI cache              | In-memory cache with configurable TTL; invalidated by daemon D-Bus signals; no SQLite in GUI                         | [09-state-flow.md](docs/architecture/09-state-flow.md)     |
| Categorization via DB + AI                    | All app-to-category mappings in `app_categories` table (seeded defaults + user edits); AI fallback for unmapped apps | [02-categorization.md](docs/features/02-categorization.md) |

## Documentation

| Document                                                   | Audience   | Contents                                                                                                              |
| ---------------------------------------------------------- | ---------- | --------------------------------------------------------------------------------------------------------------------- |
| [README.md](docs/architecture/README.md)                   | Developers | System design index: two-binary split, D-Bus interfaces, platform trait, event model, modules, state flow, deployment |
| [01-blocking.md](docs/features/01-blocking.md)             | Developers | Enforcement priorities and overlay design                                                                             |
| [02-categorization.md](docs/features/02-categorization.md) | Developers | DB-first category system, AI classification, browser tab detection                                                    |
| [03-ui-design.md](docs/features/03-ui-design.md)           | Developers | gpui-component screen layout, view models, queries                                                                    |
| [01-performance.md](docs/quality/01-performance.md)        | Developers | Zero-alloc hot path, CPU budget, async discipline                                                                     |
| [02-testing.md](docs/quality/02-testing.md)                | Developers | Given-When-Then, domain events, sociable tests                                                                        |
| [01-database.md](docs/persistence/01-database.md)          | Developers | Schema, migration policy, batch write strategy                                                                        |
| [01-roadmap.md](docs/planning/01-roadmap.md)               | Developers | Phased build plan: v1 core → v2 compositors → v3 analytics                                                            |

## Roadmap

See [01-roadmap.md](docs/planning/01-roadmap.md) for the phased build plan.
