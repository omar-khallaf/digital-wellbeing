# State Flow — Daemon Authoritative, GUI Over D-Bus

The system spans two binaries that never share memory. The daemon is the single
source of truth — it owns SQLite, evaluates policies, manages overlay state, and
writes the event log. The GUI never touches SQLite directly; it reads all data
through the daemon's D-Bus API.

## What Goes Where

| Data                            | Storage (Owner)                                                                                    | GUI Access                                      |
| ------------------------------- | -------------------------------------------------------------------------------------------------- | ----------------------------------------------- |
| Event log (focus, no-focus)     | Daemon -> SQLite                                                                                   | D-Bus method GetUsageRange()                    |
| Policies & categories           | Daemon -> SQLite                                                                                   | D-Bus method ListPolicies(), other CRUD methods |
| Daily usage (materialized view) | Daemon -> SQLite                                                                                   | D-Bus method GetUsageRange()                    |
| Block state (per-app overlays)  | Plugin (overlay state); daemon emits signal at decision time; restores via CurrentFocus on restart | D-Bus signal BlockedAppsChanged                 |
| Cache control                   | Daemon -> DB->signal                                                                               | D-Bus signals DailyUsageChanged, PolicyMutated  |

## GUI Cache Architecture

The GUI maintains an in-memory cache with no SQLite and no persistence. All data
originates from the daemon. The cache is explicitly invalidated by daemon
signals — there are no TTLs or background refreshes.

On startup, the GUI calls GetUsageRange for the last 7 days to fill the range
cache, calls ListPolicies to fill the policies cache, and subscribes to daemon
signals: BlockedAppsChanged, DailyUsageChanged, PolicyMutated.

When the user changes the time range, the GUI updates its selected range, calls
GetUsageRange with the new start and end, stores the result in the range cache,
and rebuilds the DashboardViewModel and ReportsViewModel from the cache.

When a DailyUsageChanged signal is received, the GUI clears the range cache
wholesale. The next render tick re-fetches the current selected_range via
GetUsageRange.

| Data         | Invalidation trigger        |
| ------------ | --------------------------- |
| Usage range  | `DailyUsageChanged` signal  |
| Policies     | `PolicyMutated` signal      |
| Categories   | `PolicyMutated` signal      |
| Block states | `BlockedAppsChanged` signal |

## GUI Runtime Model

The GUI process has two threads:

Thread 1 (main): gpui main loop

- Renders UI using gpui's retained-mode tree
- Polls mpsc receiver from tokio thread for ViewModel updates
- On each update: invalidate stale cache, re-render
- Sends commands via mpsc sender to tokio thread
  - e.g. CreatePolicy, DeletePolicy, ChangeDateRange

Thread 2: tokio runtime

- zbus connection to daemon's bus (resolved by bus resolution)
- Subscribe to daemon signals:
  - BlockedAppsChanged -> notify gpui thread
  - DailyUsageChanged -> invalidate range cache -> re-query
  - PolicyMutated -> invalidate policy cache -> re-query
- Method calls from gpui thread:
  - CreatePolicy(input) -> daemon
  - UpdatePolicy(id, input) -> daemon
  - DeletePolicy(id) -> daemon
  - ChangeDateRange(start, end) -> update selected_range -> re-fetch via
    GetUsageRange -> rebuild ViewModels

### Thread Safety

All cross-thread communication uses mpsc unbounded channels with Send + 'static
messages. No Arc<Mutex> shared state between threads.

The gpui thread sends user actions to the tokio thread via mpsc. The tokio
thread sends ViewModel updates back to the gpui thread via mpsc. Daemon signals
are received on the tokio thread and forwarded to the gpui thread.

## DateRange Type

The GUI uses a DateRange newtype (defined in wellbeing-core) to represent the
selected time window. DateRange carries start and end with validation that start
<= end. This makes invalid ranges unrepresentable at compile time.

Presets: 7 days, 30 days, 90 days (relative to today). Custom ranges are
constructed from explicit start/end dates via the DatePicker component in range
mode.

## Signal-Driven Invalidation

Three daemon-to-GUI signals carry cache-invalidation metadata:

| Signal             | Payload                      | Trigger                            |
| ------------------ | ---------------------------- | ---------------------------------- |
| BlockedAppsChanged | uid, app_id, blocked, reason | Block added/removed                |
| DailyUsageChanged  | uid                          | Event written -> aggregate updated |
| PolicyMutated      | uid                          | Policy created/updated/deleted     |

Signals carry minimal metadata — just enough for the GUI to know which cache
entry to invalidate. On DailyUsageChanged the GUI clears the entire range_cache
(no per-range overlap calculation). The next render tick re-fetches the current
selected_range via GetUsageRange.

## GUI ViewModel Layer

The ViewModel pattern is retained — the data source changes, but the separation
between data transformation and gpui rendering remains critical.

Each GUI screen under gui/src/dashboard/, gui/src/policies/, and
gui/src/reports/ defines ViewModels — plain Send + 'static structs holding a
pre-computed snapshot of what the render function needs. Construction happens
from the in-memory cache (not SQLite), keeping the pattern testable without gpui
initialization.

Rules:

- ViewModels are Send + 'static and contain zero gpui types.
- Each GUI screen module defines its own ViewModels.
- ViewModel construction is pure data transformation.
- The render loop follows the three-phase cycle: Collect (cache or D-Bus -> raw
  data) -> Transform (-> ViewModel) -> Render (-> gpui).
- ViewModels are rebuilt whenever AppState.selected_range changes.

Benefits: Testable data logic without gpui initialization; swappable UI
framework; no gpui imports outside gui/src/dashboard/, gui/src/policies/,
gui/src/reports/, and gui/src/appshell/.

The screen-specific view models (DashboardViewModel, PoliciesViewModel,
ReportsViewModel) and the UI components that consume them are detailed in
[ui-design.md](../features/03-ui-design.md).

## Daemon Wiring

The daemon's main.rs constructs the platform, store, and actors. Platform events
flow from the LinuxPlatform event stream through an mpsc channel to the
EnforcerActor. The EnforcerActor buffers events and evaluates policies at
minute-tick boundaries. The D-Bus server exposes methods and signals to the GUI
and plugin, reading block state from shared memory and emitting signals when
state changes.

All data access goes through D-Bus method calls that query SQLite asynchronously
within the daemon process using diesel-async.

## Root vs User UI Adaptation

The GUI detects its effective uid at startup. If the effective uid is 0, it
renders in AdminMode, showing a user selector in the title bar, a "Managed by
root" badge on root-created policies, the ability to delete or edit any policy,
and usage graphs for any user. If the effective uid is non-zero, it renders in
UserMode, showing a "Managed by admin" badge on read-only policies, edit and
delete buttons only on self-created policies, and hiding the user selector.

## GUI Startup Sequence

User launches wellbeing-gui. The GUI resolves the daemon bus via the 4-step
resolution: system present, session present, activate system, activate session.
If the daemon is found, it connects; if all steps fail, it shows a warning
banner and enters degraded mode. Next it determines its mode from getuid: root
gets AdminMode, non-root gets UserMode. Then it subscribes to daemon signals:
BlockedAppsChanged, DailyUsageChanged, PolicyMutated. It performs an initial
data fetch: ListPolicies for my_uid and GetUsageRange for the last 7 days and
my_uid. Finally it renders the dashboard.

See [10-deployment.md](./10-deployment.md) for the activation mechanism.

## GUI Graceful Degradation

| Failure                        | GUI behavior                                                                                           |
| ------------------------------ | ------------------------------------------------------------------------------------------------------ |
| Daemon not running             | Show "Daemon not running. Start with: sudo systemctl start digital-wellbeing-daemon" with retry button |
| Daemon stops mid-session       | Show warning banner, grey out data, auto-reconnect on daemon reappearance                              |
| Plugin not connected           | Show warning banner, tracking paused                                                                   |
| D-Bus method call timeout (5s) | Show error toast, retry on next render cycle                                                           |
