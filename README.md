# Digital Wellbeing

A digital wellbeing system for Wayland compositors. Tracks app usage, enforces
time limits, and helps you maintain focus — no cloud, no surveillance.

Inspired by Android's Digital Wellbeing, built for Linux desktop.

## Project Status

**v0.1 — core tracking, daemon actors, GUI, and Hyprland plugin are
implemented.** Remaining work: D-Bus interface revamp (properties→methods,
uid-from-connection), browser extension + native bridge for per-tab domain
blocking, allow-only mode, DND integration, preset blocklists, and enhanced
reports with interactive timeline.

## Architecture

The system is split into **two binaries communicating over D-Bus**, with
compositor and browser plugins for overlay enforcement and state tracking:

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
- **Compositor plugin** (`org.wellbeing.v1.Manager`) — renders app-level block
  overlays via OpenGL and emits the unified `Event` signal (with `EventTag`
  enum). Runs in the user's compositor session and resolves the daemon's bus
  using the **identical 4-step algorithm** as the GUI, so it always lands on the
  same daemon instance.
- **Native bridge** (`org.wellbeing.v1.Bridge`) + **browser extension** —
  renders per-tab domain block overlays in the browser. The bridge is a per-user
  D-Bus client that registers with the daemon (reverse discovery) and exposes a
  `DomainEvent` signal. The browser extension communicates with the bridge via
  native messaging and tracks tab/window focus (chrome.tabs, chrome.windows).
  Domain-level and app-level enforcement are independent and run in parallel.

## Workspace Layout

```
crates/
├── core/src/           # wellbeing-core: valuetypes, errors, clock, domain (shared)
├── daemon/src/
│   ├── main/           # main.rs, wiring.rs, watchdog.rs
│   ├── lib.rs          # Re-exports for integration tests
│   ├── bus_resolution.rs
│   ├── logind.rs
│   ├── signal.rs
│   ├── store/          # DbPool, migrations, schema
│   ├── platform/       # Platform trait + LinuxPlatform + ManagerClient
│   ├── dbus/           # org.wellbeing.v1.Controller server + RBAC
│   ├── blocking/       # core/ data/ domain/ (EnforcerActor)
│   ├── policy/         # core/ data/ domain/ (PolicyEngine)
│   ├── categorization/ # core.rs domain.rs (Categorizer + AI fallback)
│   └── reports/        # core/ data/ domain.rs (aggregate queries)
└── gui/
    └── src/
        ├── main/           # gpui::Application::run + bg tokio thread
        ├── app.rs          # App shell (TitleBar, TabBar, tray, user mode)
        ├── chart.rs        # Chart components
        ├── theme.rs        # Theme definitions
        ├── components.rs   # Reusable gpui components
        ├── lib.rs
        ├── dbus/           # DaemonClient (zbus proxy + signal coalescing)
        ├── cache/         # ClientCache<K,V> stale-while-revalidate
        ├── appshell/      # App shell components
        ├── dashboard/     # domain.rs timeline.rs viewmodel.rs ui/
        ├── policies/      # data.rs domain.rs ui/
        └── reports/       # data.rs domain.rs ui.rs
```

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

# Run Hyprland plugin C++ tests
cd plugins/hyprland
cmake --preset linux-host
cmake --build build/linux-host --config Debug
cd build/linux-host/wellbeing-lockdown-prefix/src/wellbeing-lockdown-build
ctest -C Debug --output-on-failure

# Lint
cargo clippy -- -D warnings
```

## Design Decisions

| Decision                                      | Rationale                                                                                                                                                       | See                                                        |
| --------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------- |
| Two binaries (daemon + GUI)                   | Root daemon owns SQLite; GUI runs unprivileged; no direct DB access in GUI; separate dep trees                                                                  | [architecture/README.md](docs/architecture/README.md)      |
| D-Bus for everything                          | Daemon, GUI, and plugin share a single IPC contract; bus daemon handles auth via SO_PEERCRED                                                                    | [architecture/README.md](docs/architecture/README.md)      |
| Plugin resolves daemon's bus (4-step)         | Plugin runs the same system→session→activate→activate algorithm as GUI to find the daemon on whatever bus it owns                                               | [04-plugin-ipc.md](docs/architecture/04-plugin-ipc.md)     |
| Per-user RBAC                                 | Daemon authenticates every D-Bus call by caller uid; root manages any user; users manage only their own                                                         | [07-rbac.md](docs/architecture/07-rbac.md)                 |
| Platform abstraction                          | OS-specific code behind `Platform` trait; Linux first, Windows later                                                                                            | [02-platform.md](docs/architecture/02-platform.md)         |
| Overlay-only enforcement                      | No process signals — compositor plugin traps input via overlay; Close Window terminates the app, does not dismiss the overlay; blocks persist until policy edit | [01-blocking.md](docs/features/01-blocking.md)             |
| Locked mode is default                        | Overlay has no dismiss action — user cannot bypass a block without changing policy                                                                              | [01-blocking.md](docs/features/01-blocking.md)             |
| Priority-ordered policy chain                 | Policies are evaluated first-match by explicit `priority` field — no implicit resolution rules                                                                  | [01-roadmap.md](docs/planning/01-roadmap.md)               |
| Categories as extra feature                   | Core tracks apps; categories are a derived grouping                                                                                                             | [02-categorization.md](docs/features/02-categorization.md) |
| ViewModel layer separates data from rendering | Each feature `ui/` constructs ViewModels from cache + signals; gpui renders ViewModels, not D-Bus actors                                                        | [09-state-flow.md](docs/architecture/09-state-flow.md)     |
| Clock trait for deterministic time            | SystemClock prod / VirtualClock test; injected into all time-dependent actors                                                                                   | [02-testing.md](docs/quality/02-testing.md)                |
| gpui-component library                        | Pre-built components (TabBar, Chart, Select, Settings, Input, Switch) for UI; avoids custom gpui layout                                                         | [03-ui-design.md](docs/features/03-ui-design.md)           |
| Stale-while-revalidate GUI cache              | In-memory cache; invalidated by daemon D-Bus signals; no SQLite in GUI                                                                                          | [09-state-flow.md](docs/architecture/09-state-flow.md)     |
| Categorization via DB + AI                    | All app-to-category mappings in `app_categories` table (seeded defaults + user edits); AI fallback for unmapped apps                                            | [02-categorization.md](docs/features/02-categorization.md) |

## Documentation

| Document                                                   | Audience   | Contents                                                                                                                      |
| ---------------------------------------------------------- | ---------- | ----------------------------------------------------------------------------------------------------------------------------- |
| [README.md](docs/architecture/README.md)                   | Developers | System design index: two-binary split, D-Bus interfaces, platform trait, event model, modules, state flow, deployment         |
| [01-blocking.md](docs/features/01-blocking.md)             | Developers | Enforcement priorities and overlay design                                                                                     |
| [02-categorization.md](docs/features/02-categorization.md) | Developers | DB-first category system, AI classification, browser tab detection                                                            |
| [03-ui-design.md](docs/features/03-ui-design.md)           | Developers | gpui-component screen layout, view models, queries                                                                            |
| [01-performance.md](docs/quality/01-performance.md)        | Developers | Zero-alloc hot path, CPU budget, async discipline                                                                             |
| [02-testing.md](docs/quality/02-testing.md)                | Developers | Given-When-Then, domain events, sociable tests                                                                                |
| [01-database.md](docs/persistence/01-database.md)          | Developers | Schema, migration policy, batch write strategy                                                                                |
| [01-roadmap.md](docs/planning/01-roadmap.md)               | Developers | Phased build plan: D-Bus revamp → browser extension domain blocking → allow-only + DND → preset blocklists → enhanced reports |

## Roadmap

See [01-roadmap.md](docs/planning/01-roadmap.md) for the phased build plan.
