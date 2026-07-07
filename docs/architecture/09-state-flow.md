# State Flow — Daemon Authoritative, GUI Over D-Bus

The system spans two binaries that never share memory. The **daemon** is the
single source of truth — it owns SQLite, evaluates policies, manages overlay
state, and writes the event log. The **GUI** never touches SQLite directly; it
reads all data through the daemon's D-Bus API.

## What Goes Where

| Data                            | Storage (Owner)                                                                                        | GUI Access                                         |
| ------------------------------- | ------------------------------------------------------------------------------------------------------ | -------------------------------------------------- |
| Event log (focus, no-focus)     | Daemon → SQLite                                                                                        | D-Bus method `GetDailyUsage()`                     |
| Policies & categories           | Daemon → SQLite                                                                                        | D-Bus method `ListPolicies()`, other CRUD methods  |
| Daily usage (materialized view) | Daemon → SQLite                                                                                        | D-Bus method `GetDailyUsage()`                     |
| Block state (per-app overlays)  | Plugin (overlay state); daemon emits signal at decision time; restores via `CurrentSession` on restart | D-Bus signal `BlockStateChanged`                   |
| Cache control                   | Daemon → DB→signal                                                                                     | D-Bus signals `DailyUsageChanged`, `PolicyMutated` |

## GUI Cache Architecture

The GUI maintains an **in-memory stale-while-revalidate cache** (no SQLite, no
persistence). All data originates from the daemon.

```
GUI startup:
  1. Call GetDailyUsage(today) → fill aggregates cache
  2. Call ListPolicies() → fill policies cache
  3. Subscribe to daemon signals: BlockStateChanged,
     DailyUsageChanged, PolicyMutated

Render loop (every 16ms frame):
  - On signal → invalidate relevant cache entry → schedule re-fetch
  - On re-fetch result → update cache → render
  - If no signal for 5s → background refresh for freshness
```

**Cache TTLs:**

| Data         | TTL          | Stale-while-revalidate   |
| ------------ | ------------ | ------------------------ |
| Daily usage  | 500ms        | Serve stale + bg refresh |
| Policies     | 5s           | Serve stale + bg refresh |
| Block states | signal-drive | Never stale (real-time)  |

## GUI Runtime Model

The GUI process has two threads:

```
Thread 1 (main): gpui::Application::run()
  │
  ├── Renders UI at 60fps using gpui's retained-mode tree
  │
  ├── Polls mpsc::UnboundedReceiver<ViewModelUpdate> from tokio thread
  │   └── On each update: invalidate stale cache, re-render
  │
  └── Sends commands via mpsc::UnboundedSender<GuiCommand> to tokio thread
      └── e.g. CreatePolicy, DeletePolicy, GrantExtension

Thread 2: tokio runtime (single-threaded, or current-thread)
  │
  ├── zbus connection to daemon's bus (resolved by `resolve_daemon_bus()`)
  │
  ├── Subscribe to daemon signals:
  │   ├── BlockStateChanged  → notify gpui thread
  │   ├── DailyUsageChanged  → invalidate usage cache → re-query
  │   └── PolicyMutated      → invalidate policy cache → re-query
  │

  ├── Periodic queries (every 1s when active):
  │   ├── GetDailyUsage(today, my_uid)
  │   └── Update usage cache
  │
  └── Method calls from gpui thread:
      ├── CreatePolicy(input) → daemon
      ├── UpdatePolicy(id, input) → daemon
      ├── DeletePolicy(id) → daemon
      └── GrantExtension(app_id) → daemon
```

### Thread Safety

```
gpui thread                       tokio thread
──────────────                    ────────────
  ViewModel updates ────mpsc────→ receive, render
  receive, update cache ←──mpsc── signals + query results
  User actions ────mpsc─────────→ D-Bus method calls
```

All cross-thread communication uses `mpsc::UnboundedChannel` with
`Send + 'static` messages. No `Arc<Mutex>` shared state between threads.

## Root vs User UI Adaptation

The GUI detects its effective uid at startup via `nix::unistd::Uid::current()`:

```
GUI startup:
  my_uid = getuid()

  if my_uid == 0:
    render_mode = AdminMode    // Can view/manage all users
    user_selector_visible = true
    policy_editor_enabled = true (all policies)
  else:
    render_mode = UserMode     // Can only see/manage self
    user_selector_visible = false
    policy_editor_enabled = true (own policies only, read-only for root-created)
```

**AdminMode UI additions:** user selector dropdown in the title bar, "Managed by
root" badge on root-created policies, ability to delete/edit any policy, usage
graphs for any user.

**UserMode UI additions:** "Managed by admin" badge on read-only policies,
edit/delete buttons only on self-created policies, user selector hidden.

## GUI Startup Sequence

```
User launches wellbeing-gui
  │
  ├── Resolve daemon bus via 4-step resolution (system present → session present
  │   → activate system → activate session; see 13-deployment-modes.md)
  │   ├── Daemon found → connect
  │   └── All steps fail → show warning banner, degraded mode
  │
  ├── Determine my mode: uid = getuid(); if uid == 0 → AdminMode else UserMode
  │
  ├── Subscribe to daemon signals: BlockStateChanged, DailyUsageChanged,
  │   PolicyMutated
  │
  ├── Initial data fetch: ListPolicies(my_uid), GetDailyUsage(today, my_uid)
  │
  └── Render dashboard
```

See [10-deployment.md](./10-deployment.md#d-bus-activation-optional) for the
activation mechanism.

## GUI Graceful Degradation

| Failure                        | GUI behavior                                                                                           |
| ------------------------------ | ------------------------------------------------------------------------------------------------------ |
| Daemon not running             | Show "Daemon not running. Start with: sudo systemctl start digital-wellbeing-daemon" with retry button |
| Daemon stops mid-session       | Show warning banner, grey out data, auto-reconnect on daemon reappearance                              |
| Plugin not connected           | Show warning banner, tracking paused                                                                   |
| D-Bus method call timeout (5s) | Show error toast, retry on next render cycle                                                           |

## D-Bus Signals (Invalidation, Not Data Delivery)

Three daemon→GUI signals carry the GUI cache-invalidation role. The notifier
itself remains the internal actor-coordination mechanism (see
[persistence/01-database.md](../persistence/01-database.md)):

| Signal              | Payload                                                | Trigger                           |
| ------------------- | ------------------------------------------------------ | --------------------------------- |
| `BlockStateChanged` | `uid: u32, app_id: String, blocked: bool, reason: u32` | Block added/removed               |
| `DailyUsageChanged` | `uid: u32`                                             | Event written → aggregate updated |
| `PolicyMutated`     | `uid: u32`                                             | Policy created/updated/deleted    |

Signals carry minimal metadata — just enough for the GUI to know which cache
entry to invalidate. The GUI then re-fetches the full data via D-Bus method
calls.

## GUI ViewModel Layer

The ViewModel pattern is **retained** — the data source changes, but the
separation between data transformation and gpui rendering remains critical.

Each GUI screen under `gui/src/screens/<feature>/` defines **ViewModels** —
plain `Send + 'static` structs holding a pre-computed snapshot of what the
render function needs. Construction happens from the in-memory cache (not
SQLite), keeping the pattern testable without gpui initialization.

See
[DashboardViewModel in 03-ui-design.md](../features/03-ui-design.md#dashboardviewmodel)
for the canonical definition. The full struct carries date range, bar chart
data, per-app and per-category pie slices, top-apps list, and block cards.

**Rules:**

- ViewModels are `Send + 'static` and contain **zero gpui types**.
- Each GUI screen module defines its own ViewModels.
- ViewModel construction is pure data transformation.
- The render loop follows the three-phase cycle: **Collect** (cache or D-Bus →
  raw data) → **Transform** (→ ViewModel) → **Render** (→ gpui).

**Benefits:** Testable data logic without gpui initialization; swappable UI
framework; no gpui imports outside `gui/src/screens/` and `gui/src/ui/`.

The screen-specific view models (`DashboardViewModel`, `PoliciesViewModel`) and
the UI components that consume them are detailed in
[ui-design.md](../features/03-ui-design.md).

## Daemon Wiring

The daemon's actor wiring:

```rust
// In daemon/main.rs:
let pool = StoreBuilder::new(db_path).build().await.unwrap();
let (notifier, notifier_rx) = ReactiveNotifier::new();

// ReactiveNotifier now drives D-Bus signal emission instead of
// in-process watch channel notifications.

let (approved_events_tx, approved_events_rx) = mpsc::channel(32);

let (platform, event_stream) = LinuxPlatformBuilder::new().build().await.unwrap();

let tracker = TrackerActor::new(approved_events_rx, pool.clone(), notifier.clone());
let enforcer = EnforcerActor::new(
    event_stream, approved_events_tx,
    pool.clone(), notifier.clone(),
);

// D-Bus server actor — exposes methods/signals to GUI, forwards
// commands to enforcer.
let dbus = DaemonDbusActor::new(
    pool.clone(), notifier_rx,
    enforcer.block_state_tx,
);
```

The `ReactiveNotifier` emits three signals on the daemon's D-Bus API; a `watch`
channel for `BlockState` drives `BlockStateChanged` emission. All data access
goes through D-Bus method calls that query SQLite synchronously within the
daemon process.
