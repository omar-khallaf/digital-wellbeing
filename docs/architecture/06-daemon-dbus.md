# Controller D-Bus Interface — `org.wellbeing.v1.Controller`

The daemon exposes a D-Bus interface (`org.wellbeing.v1.Controller`) for the GUI
and compositor plugin to query and observe state. In **system mode** (root) the
daemon is on the **system bus**; in **session mode** (non-root) it is on the
**session bus** — see [13-deployment-modes.md](./13-deployment-modes.md) for
bus/scope selection. All methods authenticate the caller via D-Bus credentials
(`uid`) and enforce RBAC (see [07-rbac.md](./07-rbac.md)). The plugin side of
the same bus is documented in [04-plugin-ipc.md](./04-plugin-ipc.md).

## Interface Overview

### Usage Queries

| Method          | Input                                        | Output            | Description                                                              |
| --------------- | -------------------------------------------- | ----------------- | ------------------------------------------------------------------------ |
| `GetUsageRange` | `start_date: s`, `end_date: s`, `user_id: u` | `summaries: a(v)` | Usage grouped by day for the inclusive date range. Dates are `%Y-%m-%d`. |

`GetUsageRange` is the primary data-fetch method for the GUI. It returns
`Vec<DailySummary>` where each `DailySummary` groups all `DailyUsageEntry`
records for a single date. The GUI consumes this directly for both the Dashboard
and Reports screens.

### Policy CRUD

| Method         | Input                                             | Output           | Description               |
| -------------- | ------------------------------------------------- | ---------------- | ------------------------- |
| `ListPolicies` | `filter_owner: u` (0=caller; non-zero: root only) | `policies: a(v)` | List policies             |
| `CreatePolicy` | `input: v`                                        | `id: t`          | Create a new policy       |
| `UpdatePolicy` | `id: t`, `input: v`                               | —                | Update an existing policy |
| `DeletePolicy` | `id: t`                                           | —                | Delete a policy           |

### Plugin Registration

| Method           | Input | Description                                                                                                                                                                                                                                       |
| ---------------- | ----- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `RegisterPlugin` | —     | Called by compositor plugins on startup. Daemon authenticates via `SO_PEERCRED` uid, learns the plugin's unique bus name from `header.sender()`, subscribes to plugin signals. Open to any caller — identity comes from the kernel, not the call. |

### Block State (Declarative)

| Property       | Type          | Access | Description                                                                                                                                                                                                                                                           |
| -------------- | ------------- | ------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `ActiveBlocks` | `a(s(tutau))` | read   | Currently blocked apps. Each entry: `{app_id, policy_id, blocked_since, reason, available_actions}`. The canonical source of truth for blocking state. Consumed by the compositor plugin (reads on startup for crash recovery) and GUI (reads for dashboard display). |

The `ActiveBlocks` property is the sole source of truth for which apps are
blocked and why. The daemon writes to it; all consumers (plugin, GUI) read from
it. The plugin subscribes to `BlockStateChanged` for live updates but falls back
to `ActiveBlocks` for initial state and reconciliation.

### Signals

| Signal              | Fields                                       | When                                                                                                                |
| ------------------- | -------------------------------------------- | ------------------------------------------------------------------------------------------------------------------- |
| `BlockStateChanged` | `{uid: u, app_id: s, blocked: b, reason: u}` | An app is blocked or unblocked. Consumed by plugin (real-time overlay sync) and GUI (dashboard cache invalidation). |
| `DailyUsageChanged` | `{uid: u}`                                   | Daily usage data mutated. Consumed by GUI for cache invalidation.                                                   |
| `PolicyMutated`     | `{uid: u}`                                   | Policy created, updated, or deleted. Consumed by GUI for cache invalidation.                                        |

### D-Bus Message Size Limits

All D-Bus calls are local AF_UNIX. Default zbus message limit is 128 MB. Typical
payload sizes:

| Call            | Typical size   | Max expected |
| --------------- | -------------- | ------------ |
| `ListPolicies`  | 200 B – 2 KB   | 50 KB        |
| `GetUsageRange` | 10 KB – 100 KB | 1 MB         |
| `ActiveBlocks`  | 100 B – 1 KB   | 10 KB        |
| Signals         | < 200 B        | 1 KB         |

All well within limits.

### D-Bus Error Mapping

Domain errors from the daemon's business logic MUST be mapped to well-known
D-Bus error replies, not returned as generic
`org.freedesktop.DBus.Error.Failed`.

| Domain error variant                  | D-Bus error name                          | HTTP analogy |
| ------------------------------------- | ----------------------------------------- | ------------ |
| `PolicyNotFound`                      | `org.wellbeing.Error.PolicyNotFound`      | 404          |
| `PolicyConflict` (duplicate)          | `org.wellbeing.Error.PolicyConflict`      | 409          |
| `PermissionDenied`                    | `org.freedesktop.DBus.Error.AccessDenied` | 403          |
| `ValidationError` (newtype rejection) | `org.wellbeing.Error.InvalidArgument`     | 400          |
| `StorageError` (DB connection)        | `org.freedesktop.DBus.Error.Failed`       | 500          |
| `PluginNotConnected`                  | `org.wellbeing.Error.PluginNotConnected`  | 503          |
| `InternalError`                       | `org.wellbeing.Error.Failed`              | 500          |

Each D-Bus method handler catches domain errors and converts them to
`zbus::Error` with the mapped name. This ensures D-Bus clients (the GUI, CLI
tools) can discriminate error types programmatically by matching on the error
name string, rather than parsing generic failure messages.

## GUI D-Bus Client Architecture

The GUI maintains an **in-memory stale-while-revalidate cache** and talks
exclusively to the daemon over D-Bus (never directly to SQLite). See
[09-state-flow.md](./09-state-flow.md#gui-cache-architecture) for the GUI-side
cache lifecycle, TTLs, and runtime model.

### Signal Coalescing

D-Bus signals can fire rapidly (e.g., `BlockStateChanged` for every app,
`DailyUsageChanged` on every focus switch). The GUI coalesces them via atomic
dirty flags: each signal handler sets a flag, and a drain function collects all
dirty flags into a single notification struct. The render loop checks coalesced
notifications between frames and invalidates the appropriate cache entries.

### Client Cache

The GUI maintains typed time-to-live caches (`ClientCache<K, V>`) for each D-Bus
response type. Each cache entry stores the deserialized value plus a fetch
timestamp. On read, the cache returns the value if still within the TTL;
otherwise returns `None` (the caller re-fetches from the daemon). On signal
reception, the relevant cache keys are invalidated so the next render cycle
re-fetches fresh data.

The usage cache is keyed by `"range:{start}:{end}:{uid}"`. On
`DailyUsageChanged` the entire usage cache is cleared (no per-range overlap
calculation). The next render tick re-fetches the current `selected_range`.

The cache is purely in-memory — no SQLite, no persistence.

## References

- [04-plugin-ipc.md](./04-plugin-ipc.md) — plugin IPC, `ActiveBlocks`
  consumption
- [05-daemon-auth.md](./05-daemon-auth.md) — trust model (no crypto needed)
- [07-rbac.md](./07-rbac.md) — `SO_PEERCRED` uid authentication model
- [09-state-flow.md](./09-state-flow.md) — GUI cache architecture, DateRange,
  signal invalidation
