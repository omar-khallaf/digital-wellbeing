# Database Design

## Stack

The persistence layer uses diesel with the diesel-async SQLite backend, running
in WAL mode so that read queries from the dashboard and write operations from
feature actors can proceed concurrently. WAL mode is enabled at connection open
with the SQLite PRAGMAs journal_mode=WAL and synchronous=NORMAL.

A connection pool backed by AsyncSqliteConnection hands out connections to
feature data modules; the pool is constructed once at startup and shared for the
process lifetime.

---

## Store Module (`store/`)

The `store/` module owns the connection lifecycle, migration runner, and pool
type. Dependency flow is: `store/` depends on diesel, diesel-async, and the
migration files, while feature `data/` modules depend only on `store/` for the
pool type — never on diesel directly.

### Connection Lifecycle

At startup the builder opens a SQLite connection in WAL mode, runs any pending
migrations against that connection, and then wraps the live connection in a
pool. Feature actors call `pool.get()` to write events and read policy state.
The gpui dashboard also reads through the same pool to render usage aggregates
and event history.

On shutdown each actor cancels its tokio CancellationToken. Before the pool is
dropped, close events such as Slept or LoggedOut are flushed to close any open
focus intervals. The pool drain then closes all connections and checkpoints the
WAL.

### Migration Runner

Migrations live in `store/migrations.rs` and use diesel's embedded migrations
with SQL files under `migrations/`. At startup the builder runs all pending
migrations against the new connection before handing out a pool. The project
follows a forward-only, additive migration policy managed by the diesel CLI:
every migration only adds new tables, columns, or indexes. No down migrations
are written. Rollback is achieved by deploying the previous binary. Failed
migrations cause the process to exit; on restart diesel re-runs the failed
migration because SQLite DDL is transactional for most DDL statements.

---

## Schema

### `events` — Append-Only Event Log

Eight event types cover every focus switch and state change. Every focus switch
or state change writes exactly one row.

The table uses STORED generated columns for timestamp and app_id, materialized
from the JSON payload at insert time via json_extract. This keeps all field data
in the JSON payload as the single source of truth while making the extracted
values physically present in the row so indexes on timestamp and app_id work
without function-wrapped lookups. Interval accumulation is handled in Rust via
`accumulate_daily_usage` called in the same transaction as the event INSERT, so
business logic stays in application code instead of SQL triggers.

Payload field names are shortened to one character (`t` for timestamp, `a` for
app_id) to reduce per-row JSON storage overhead. At the expected event volume
the shortened keys save several megabytes per year in JSON key names alone.

A CHECK constraint enforces that the payload must contain string fields `t` and
`a` when event_type indicates WindowFocused, and only string `t` with null or
absent `a` for Unfocused and the close-event types. No other event_type values
are accepted.

Interval computation happens at write time. Tracked time for an app equals
the wall-clock span from `WindowFocused` to the next close event (`Unfocused`,
`Locked`, `LoggedOut`, `Slept`, `ShutDown`). Idle spans are included in tracked
time; the GUI can derive idle breakdown from the raw `Idle`/`Resumed` event
sequence if needed.

The id column uses AUTOINCREMENT because it serves as an ordering token for the
reactive watch channel; consumers track last seen event id to avoid
re-processing known events.

Timestamps are stored as YYYY-MM-DD HH:MM:SS in UTC, space-separated, with no
timezone offset. This single format gives lexicographic ordering equal to
chronological ordering because all values are UTC, and SQLite date functions
parse it directly for query-time duration math.

### `daily_usage` — Materialized Daily Usage Per App

This materialized view holds per-app daily usage totals maintained by
application-level transactions that wrap each event INSERT in an explicit
BEGIN/COMMIT pair. The same transaction calls `accumulate_daily_usage` to update
the materialized view, so the event write and the usage update are atomic.

Focus state is maintained in-memory by the `EnforcerActor` as a `HashMap` per
user, never persisted in the database. The events log is the source of truth;
the in-memory state is just the live accumulator.

`accumulate_daily_usage` computes elapsed minutes from the focus state
(wall-clock time including idle), derives the date from the focus start time,
and upserts into `daily_usage` within the same transaction as the event INSERT.
The extended flag is set by the `EnforcerActor` after granting extra time via a
direct UPDATE; it is not set by a trigger.

Application-level transactions provide the same atomicity that SQL triggers
would while keeping business logic in Rust. If the daemon crashes
mid-transaction the entire operation rolls back, preserving crash consistency.

### `policies` — Blocking, Time Limit & Notify Rules

This table stores every active policy. Each policy targets either a category or
a specific app, never both; an exclusive arc CHECK prevents orphan targeting.
The kind column uses an integer that maps one-to-one with the PolicyKind Rust
enum.

Block kind has no time limit; TimeLimit and Notify kinds require a positive
time_limit_minutes. The extra_minutes column configures how much extra time the
user receives on an Extend action for TimeLimit policies. The
notification_repeat_interval_minutes column controls re-notification cadence for
Notify policies; NULL means notify once, a positive value means repeat at that
interval in minutes.

Schedule columns define when the policy is active. Both schedule_start_hour and
schedule_end_hour are either both present or both NULL; when present they are
integers from 0 to 23. schedule_days is a JSON array of weekday numbers where 0
is Sunday through 6 is Saturday, and an empty array means all days.

RBAC is enforced at the row level through owner_id, which scopes policies to a
user, and created_by, which records authorship.

### `categories` — User-Defined Groupings

This table holds the category roster with a unique name constraint and optional
display metadata such as color and icon. Built-in categories are seeded at first
run.

### `app_categories` — App-to-Category Mappings

This table is the single source of truth for app categorization. Every row is
authoritative, whether seeded as a default or edited by the user. The user_id
column distinguishes system-global defaults (user_id=0) seeded at migration time
from per-user overrides (user_id=N). Resolution checks the user-specific row
first; if no row exists it falls back to the global default.

Category_id may be null; when it is the categorizer falls through to AI
classification and ultimately Uncategorized rather than using a NULL FK.

---

## Reactive System

Events form the reactive data surface. When a new event is written, consumers
must re-evaluate their timers, UI aggregates, and block state. A tagged
notification enum backed by a tokio watch channel lets consumers skip irrelevant
work. The channel carries only the latest variant, so concurrent EventWritten
and PolicyMutated notifications coalesce to the most recent. Consumers treat any
notification as a signal that state may have changed and should re-check or
invalidate caches.

EventWritten is published on every events INSERT. PolicyMutated is published on
every policies or categories INSERT/UPDATE/DELETE and on every app_categories
INSERT/UPDATE/DELETE because categorization changes can alter policy evaluation.

The tracker, enforcer, and dashboard each hold a watch::Receiver cloned from the
notifier. On notification they check the variant: the tracker updates active
window state on EventWritten and re-evaluates limits on PolicyMutated; the
enforcer re-evaluates blocks on EventWritten and checks whether a policy change
lifts an active block on PolicyMutated; the dashboard invalidates its
stale-while-revalidate cache on any notification so the next render frame
re-fetches.

---

## Query Patterns

### Daily Usage for Policy Evaluation

The policy engine reads daily_usage by date and app_id to obtain the total
minutes and extended flag. This is a point lookup on the materialized table. The
calling code constructs the appropriate domain type from the result depending on
whether the policy is TimeLimit or Notify.

### Daily Usage Report for Dashboard

The dashboard reads total minutes plus extended flag per app for a given date by
scanning daily_usage filtered by date. The result is an ordered list of app
totals.

### Last Event for Boot Reconciliation

The daemon reads the most recent event from the events table at startup to
reconcile with the plugin state. If the last event indicates an open interval,
the daemon continues or closes that interval as appropriate on the next real
event.

### Historical Report from Raw Events

For reports spanning longer time ranges, the daemon reads all events within a
date range ordered by timestamp. These infrequent OLAP-style queries scan raw
events because daily_usage does not retain historical resolution beyond its
retention window.

### Open Interval Tracking

The currently focused app is tracked in-memory by the `EnforcerActor` as a
`HashMap` per user. The dashboard and policy engine query this actor state
directly rather than hitting the database, because the in-memory state reflects
the latest focus event without waiting for a transaction round-trip.

For historical consistency checks at startup after a process restart, the last
event in the events table is used. If the most recent event is WindowFocused the
interval is still open; if it is any close event the interval has already
closed. A tail Idle means the interval is open but paused.

---

## Batch Write Strategy

Events are written on demand: each window focus switch or Unfocused produces
exactly one row. Write frequency is bounded by user interaction rate and well
within SQLite's single-insert throughput in WAL mode.

### Background Prune

A background task runs every hour to enforce retention. Raw events older than
ninety days are deleted in batches of five hundred to avoid WAL bloat and long
table locks. Daily usage older than ninety days is pruned the same way because
the table is tiny and the date column sorts lexicographically. The two tables
are pruned independently in the same loop.

### Power State-Aware Flush

When the system is about to suspend, hibernate, or shut down, the open focus
interval must be closed so wall-clock time during the power state change is not
counted. A PowerStateWatcher subscribes to systemd-logind PrepareForSleep and
PrepareForShutdown signals via D-Bus. On PrepareForSleep(TRUE) it emits a real
Slept event; on PrepareForShutdown(TRUE) it emits a real ShutDown event. These
are genuine occurrences, so the event log stays truthful and the interval is
simply closed by the existing accumulation logic.

Session lifecycle events such as Locked and LoggedOut are emitted by the same
watcher from logind Session Lock and session-removed signals. If the flush
fails, the error is logged and the D-Bus delay inhibitor is released anyway.
Losing a few seconds of usage data is acceptable; blocking a power state change
is not.

### Process Termination Handling

Logout does not trigger a logind PrepareForShutdown signal. The display manager
terminates the session compositor, which sends SIGHUP or SIGTERM to child
processes. A tokio signal handler hooks SIGTERM and SIGINT to emit a real
LoggedOut event before the process exits. This covers logout, Ctrl+C in a
terminal, systemctl --user stop, and terminal close. The handler accesses the
actor focus state and user_id needed to close the open interval, then cancels
the tokio runtime to stop all actors cleanly.
