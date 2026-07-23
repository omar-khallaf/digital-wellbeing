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
| Block state (per-app overlays)  | Plugin (overlay state); daemon emits signal at decision time; restores via CurrentFocus on restart | D-Bus signal BlockStateChanged                  |
| Cache control                   | Daemon -> DB->signal                                                                               | D-Bus signals DailyUsageChanged, PolicyMutated  |

## GUI Cache Architecture

The GUI maintains an in-memory stale-while-revalidate cache with no SQLite and
no persistence. All data originates from the daemon.

On startup, the GUI calls GetUsageRange for the last 7 days to fill the range
cache, calls ListPolicies to fill the policies cache, and subscribes to daemon
signals: BlockStateChanged, DailyUsageChanged, PolicyMutated.

When the user changes the time range, the GUI updates its selected range, calls
GetUsageRange with the new start and end, stores the result in the range cache,
and rebuilds the DashboardViewModel and ReportsViewModel from the cache.

When a DailyUsageChanged signal is received, the GUI clears the range cache
wholesale. The next render tick re-fetches the current selected_range via
GetUsageRange.

Cache TTLs:

| Data         | TTL          | Stale-while-revalidate   |
| ------------ | ------------ | ------------------------ |
| Usage range  | 500ms        | Serve stale + bg refresh |
| Policies     | 5s           | Serve stale + bg refresh |
| Block states | signal-drive | Never stale (real-time)  |

## GUI Runtime Model

The GUI process has two threads:

Thread 1 (main): gpui main loop | |-- Renders UI at 60fps using gpui's
retained-mode tree | |-- Polls mpsc receiver from tokio thread | |-- On each
update: invalidate stale cache, re-render | |-- Sends commands via mpsc sender
to tokio thread |-- e.g. CreatePolicy, DeletePolicy, GrantExtension,
ChangeDateRange

Thread 2: tokio runtime | |-- zbus connection to daemon's bus (resolved by
resolve_daemon_bus()) | |-- Subscribe to daemon signals: | |-- BlockStateChanged
-> notify gpui thread | |-- DailyUsageChanged -> invalidate range cache ->
re-query | |-- PolicyMutated -> invalidate policy cache -> re-query | |--
Periodic queries (every 1s when active): | |--
GetUsageRange(selected_range.start, selected_range.end, my_uid) | |-- Update
range cache | |-- Method calls from gpui thread: |-- CreatePolicy(input) ->
daemon |-- UpdatePolicy(id, input) -> daemon |-- DeletePolicy(id) -> daemon |--
GrantExtension(app_id) -> daemon |-- ChangeDateRange(start, end) -> update
selected_range -> re-fetch via GetUsageRange -> rebuild ViewModels

### Thread Safety

All cross-thread communication uses mpsc unbounded channels with Send + static
messages. No Arc<Mutex> shared state between threads.

gpui thread tokio thread

---

ViewModel updates ----> receive, render receive, update cache <--- signals +
query results User actions -----> D-Bus method calls

## DateRange Type

The GUI uses a DateRange newtype (defined in wellbeing-core) to represent the
selected time window. DateRange carries start and end with validation that start
<= end. This makes invalid ranges unrepresentable at compile time.

Presets: 7 days, 30 days, 90 days (relative to today). Custom ranges are
constructed from explicit start/end dates via the DatePicker component in range
mode.

## Signal-Driven Invalidation

Three daemon-to-GUI signals carry cache-invalidation metadata:

| Signal            | Payload                      | Trigger                            |
| ----------------- | ---------------------------- | ---------------------------------- |
| BlockStateChanged | uid, app_id, blocked, reason | Block added/removed                |
| DailyUsageChanged | uid                          | Event written -> aggregate updated |
| PolicyMutated     | uid                          | Policy created/updated/deleted     |

Signals carry minimal metadata — just enough for the GUI to know which cache
entry to invalidate. On DailyUsageChanged the GUI clears the entire range_cache
(no per-range overlap calculation). The next render tick re-fetches the current
selected_range via GetUsageRange.

## GUI ViewModel Layer

The ViewModel pattern is retained — the data source changes, but the separation
between data transformation and gpui rendering remains critical.

Each GUI screen under gui/src/screens/<feature>/ defines ViewModels — plain
Send + 'static structs holding a pre-computed snapshot of what the render
function needs. Construction happens from the in-memory cache (not SQLite),
keeping the pattern testable without gpui initialization.

Rules:

- ViewModels are Send + 'static and contain zero gpui types.
- Each GUI screen module defines its own ViewModels.
- ViewModel construction is pure data transformation.
- The render loop follows the three-phase cycle: Collect (cache or D-Bus -> raw
  data) -> Transform (-> ViewModel) -> Render (-> gpui).
- ViewModels are rebuilt whenever AppState.selected_range changes.

Benefits: Testable data logic without gpui initialization; swappable UI
framework; no gpui imports outside gui/src/screens/ and gui/src/ui/.

The screen-specific view models (DashboardViewModel, PoliciesViewModel,
ReportsViewModel) and the UI components that consume them are detailed in
[ui-design.md](../features/03-ui-design.md).

## Daemon Wiring

The daemon's actor wiring constructs a StoreBuilder to obtain the database pool,
then exposes three D-Bus signals. Approved events flow through an mpsc channel
from the event stream to the enforcer. The Linux platform builder provides the
event stream. The enforcer actor receives the event stream, the pool, and the
signal sender. The D-Bus server actor exposes methods and signals to the GUI,
forwards commands to the enforcer, and receives the enforcer's block state
sender.

All data access goes through D-Bus method calls that query SQLite synchronously
within the daemon process.

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
BlockStateChanged, DailyUsageChanged, PolicyMutated. It performs an initial data
fetch: ListPolicies for my_uid and GetUsageRange for the last 7 days and my_uid.
Finally it renders the dashboard.

See [10-deployment.md](./10-deployment.md) for the activation mechanism.

## GUI Graceful Degradation

| Failure                        | GUI behavior                                                                                           |
| ------------------------------ | ------------------------------------------------------------------------------------------------------ |
| Daemon not running             | Show "Daemon not running. Start with: sudo systemctl start digital-wellbeing-daemon" with retry button |
| Daemon stops mid-session       | Show warning banner, grey out data, auto-reconnect on daemon reappearance                              |
| Plugin not connected           | Show warning banner, tracking paused                                                                   |
| D-Bus method call timeout (5s) | Show error toast, retry on next render cycle                                                           |
