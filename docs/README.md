# Documentation

Design and reference documentation for the Digital Wellbeing system, grouped by
concern. All docs are Markdown; links are relative to this directory unless
noted.

## Directories

| Directory                          | Contents                                                                                                                                                                 |
| ---------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| [`architecture/`](./architecture/) | System & IPC design: the two-binary split, D-Bus interfaces + RBAC, the `Platform` trait, event model, workspace layout, deployment, and the phased implementation plan. |
| [`features/`](./features/)         | Per-feature design: `01-blocking` (overlay-only enforcement), `02-categorization` (DB-first category system + AI fallback), `03-ui-design` (gpui screens, view models).  |
| [`persistence/`](./persistence/)   | `01-database`: SQLite schema, migration policy, and the per-event / bulk write strategy.                                                                                 |
| [`quality/`](./quality/)           | Cross-cutting engineering: `01-performance` (zero-alloc hot path, CPU budget, async) and `02-testing` (philosophy).                                                      |
| [`planning/`](./planning/)         | `01-roadmap`: planned features and explicit non-goals.                                                                                                                   |

## How the docs relate

- `architecture/` is the system-design hub. Feature, persistence, quality, and
  planning docs are siblings that focus on a single concern each; they link back
  into `architecture/` for shared context (traits, D-Bus contracts, RBAC).
- The daemon is **headless**: no `ui/` directories live in daemon feature trees.
  GUI lives in the `gui/` crate (`gui/src/screens/<feature>/`).
- `blocking/` has no GUI screen — it is overlay-only enforcement, and the
  overlay is rendered by the compositor plugin, not gpui.

See the repo-root [`README.md`](../README.md) for the build, module layout, and
the consolidated Documentation table.
