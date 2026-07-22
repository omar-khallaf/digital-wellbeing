# Workspace Implementation Plan (Design → Code)

Phased build plan. The [overview](./README.md) describes the target design; this
file tracks progress phase by phase.

## Phase A: Foundation (workspace + shared types)

| Step | File(s)             | What                                             |
| ---- | ------------------- | ------------------------------------------------ |
| A1   | `Cargo.toml`        | Workspace with core/daemon/gui members           |
| A2   | `crates/core/*`     | Newtypes, Error, Clock trait, domain D-Bus types |
| A3   | `migrations/up.sql` | Updated schema with user_id/created_by/owner_id  |

Status: **DONE** (A1–A3 completed in initial workspace restructuring)

## Phase B: Daemon core (store + platform + D-Bus server)

| Step | File(s)                  | What                                         |
| ---- | ------------------------ | -------------------------------------------- |
| B1   | `daemon/src/store/*`     | DbPool, StoreBuilder, migrations, schema.rs  |
| B2   | `daemon/src/platform/*`  | Platform trait, LinuxPlatform, ManagerClient |
| B3   | `daemon/src/dbus/mod.rs` | org.wellbeing.v1.Controller server + RBAC    |

Status: **B1 done** (store), B2–B3 **in progress**.

## Phase C: Daemon actors (tracking, policy engine, enforcer)

| Step | File(s)                       | What                                         |
| ---- | ----------------------------- | -------------------------------------------- |
| C1   | `daemon/src/tracking/*`       | TrackerActor — event persistence             |
| C2   | `daemon/src/policy/*`         | PolicyEngine — evaluate(), domain types      |
| C3   | `daemon/src/blocking/*`       | EnforcerActor — gate-first pipeline, overlay |
| C4   | `daemon/src/categorization/*` | Category resolution + AI fallback            |
| C5   | `daemon/src/reports/*`        | Aggregate queries for history/export         |
| C6   | `daemon/src/main.rs`          | Wiring all actors + D-Bus server             |

## Phase D: GUI

| Step | File(s)                | What                                                    |
| ---- | ---------------------- | ------------------------------------------------------- |
| D1   | `gui/src/dbus/mod.rs`  | DaemonClient + SignalCoalescer, signal subscription     |
| D2   | `gui/src/cache/mod.rs` | ClientCache (stale-while-revalidate)                    |
| D3   | `gui/src/main.rs`      | gpui::run + background tokio thread                     |
| D4   | `gui/src/app.rs`       | App shell (TitleBar, TabBar, tray, user mode detection) |
| D5   | `gui/src/dashboard.rs` | Dashboard screen (usage charts)                         |
| D6   | `gui/src/policies.rs`  | Policies screen (CRUD, RBAC-aware)                      |
| D7   | `gui/src/reports.rs`   | Reports screen (history, export)                        |

## Phase E: Plugin migration

| Step | File(s)                  | What                            |
| ---- | ------------------------ | ------------------------------- |
| E1   | `plugins/hyprland/src/*` | Change session bus → system bus |
| E2   | `plugins/hyprland/src/*` | Add CurrentFocus property       |
| E3   | `deploy/*.conf`          | D-Bus system policy files       |

## Phase F: Deployment

| Step | File(s)                                           | What               |
| ---- | ------------------------------------------------- | ------------------ |
| F1   | `deploy/systemd/digital-wellbeing-daemon.service` | systemd unit       |
| F2   | `deploy/*.conf`                                   | D-Bus policy files |
| F3   | `Makefile` or `justfile`                          | Install targets    |
