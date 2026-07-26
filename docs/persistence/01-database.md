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

Nine event types cover every focus switch and state change. Every focus switch
or state change writes exactly one row. The events table is the **single source
of truth** — it holds raw, denormalized `app` and `title` strings (not FKs).

| Column     | Type    | Notes                                                                          |
| ---------- | ------- | ------------------------------------------------------------------------------ |
| id         | INTEGER | AUTOINCREMENT primary key                                                      |
| event_type | INTEGER | 0=WindowFocused, 1=Unfocused, 2=Idle, …                                        |
| user_id    | INTEGER | UID of the user this event belongs to                                          |
| timestamp  | BIGINT  | Epoch milliseconds (UTC) — indexed for queries                                 |
| app        | TEXT?   | Raw app identifier string (e.g. "firefox"). Non-null for WindowFocused/Blocked |
| title      | TEXT?   | Raw window title. Non-null for WindowFocused/Blocked. NULL for power events.   |

Event-type invariants are enforced via CHECK constraints:

| Code | Event         | app  | title | Description                    |
| ---- | ------------- | ---- | ----- | ------------------------------ |
| 0    | WindowFocused | ✓    | ✓     | An app window gained focus     |
| 1    | Unfocused     | NULL | NULL  | No window is focused (desktop) |
| 2    | Idle          | —    | —     | User became idle               |
| 3    | Resumed       | —    | —     | User resumed from idle         |
| 4    | Slept         | NULL | NULL  | System entered sleep           |
| 5    | ShutDown      | NULL | NULL  | System shut down               |
| 6    | Locked        | NULL | NULL  | Session locked                 |
| 7    | LoggedOut     | NULL | NULL  | User logged out                |
| 8    | WindowBlocked | ✓    | ✓     | Window was blocked by policy   |

The event type constants are shared across the daemon and GUI via
`wellbeing_core::event_types`.

Interval computation happens at write time. Tracked time for an app equals the
wall-clock span from `WindowFocused` to the next close event (`Unfocused`,
`Locked`, `LoggedOut`, `Slept`, `ShutDown`). Idle spans are included in tracked
time; the GUI can derive idle breakdown from the raw `Idle`/`Resumed` event
sequence if needed.

### `apps` — Global App Registry

A normalized projection of known app identifiers. Populated on-demand via upsert
when a policy targets a non-existing app or when aggregating daily usage from
events. Global across all users — "firefox" is the same app regardless of who
launches it.

| Column | Type    | Notes                                   |
| ------ | ------- | --------------------------------------- |
| id     | INTEGER | AUTOINCREMENT primary key               |
| app_id | TEXT    | App identifier (e.g. "firefox"), UNIQUE |

No display name column — the raw `app_id` string is the canonical label.
Per-user display overrides can be set via `app_categories.icon_path`.

Upsert pattern:

```sql
INSERT INTO apps (app_id) VALUES ('firefox') ON CONFLICT(app_id) DO NOTHING;
```

### `daily_usage` — Materialized Daily Usage Per App

This materialized view holds per-app daily usage totals, referencing the `apps`
table via FK (`apps_id`). Maintained by application-level transactions that wrap
each event INSERT in an explicit BEGIN/COMMIT pair. The same transaction calls
`accumulate_daily_usage` to update the materialized view, so the event write and
the usage update are atomic.

| Column        | Type    | Notes                                       |
| ------------- | ------- | ------------------------------------------- |
| date          | TEXT    | Calendar date (%Y-%m-%d)                    |
| user_id       | INTEGER | UID                                         |
| apps_id       | INTEGER | FK to `apps.id`                             |
| closed_millis | INTEGER | Tracked wall-clock time in closed intervals |
| open_millis   | INTEGER | Tracked wall-clock time in open interval    |

Focus state is maintained in-memory by the `EnforcerActor` as a `HashMap` per
user, never persisted in the database.

`accumulate_daily_usage` computes elapsed milliseconds from the focus state,
derives the date from the focus start time, and upserts into `daily_usage`
within the same transaction as the event INSERT. Block state is managed
declaratively through the daemon's BlockedApps D-Bus property, not through the
events table or daily_usage.

### `daily_usage_by_title` — Per-App, Per-Title Usage Breakdown

Same structure as `daily_usage` but broken down by window title for finer
granularity.

| Column        | Type    | Notes                                       |
| ------------- | ------- | ------------------------------------------- |
| date          | TEXT    | Calendar date (%Y-%m-%d)                    |
| user_id       | INTEGER | UID                                         |
| apps_id       | INTEGER | FK to `apps.id`                             |
| title         | TEXT    | Window title (up to 1024 chars)             |
| closed_millis | INTEGER | Tracked wall-clock time in closed intervals |
| open_millis   | INTEGER | Tracked wall-clock time in open interval    |

### `policies` — Priority-Ordered Rule Chain

This table stores every policy as a priority-ordered rule. Evaluation is
first-match: sort by `priority` ascending, return the first policy whose target
matches and whose schedule is active. No match means unrestricted.

| Column             | Type    | Notes                                                    |
| ------------------ | ------- | -------------------------------------------------------- |
| id                 | INTEGER | Primary key                                              |
| name               | TEXT    | Human-readable label, non-empty                          |
| priority           | INTEGER | Lower = evaluated first. Default 100.                    |
| effect             | INTEGER | 0=Allow, 1=Block, 2=TimeLimit, 3=Notify                  |
| apps_id            | INTEGER | FK to `apps.id` — non-null for App target                |
| category_id        | INTEGER | FK to `categories.id` — non-null for Category target     |
| domain_pattern     | TEXT    | Domain pattern — non-null for Domain target              |
| time_limit_minutes | INTEGER | Required for TimeLimit/Notify, forbidden for Allow/Block |
| schedule_json      | TEXT    | JSON array of TimeWindow; `[]` = always active           |
| user_id            | INTEGER | The user this policy applies to                          |
| created_by         | INTEGER | The caller UID that created this policy (0 = root)       |

**Target discrimination:** Exactly one of `apps_id`, `category_id`, or
`domain_pattern` is non-null. When all three are null the target is `Any`
(matches everything).

**Integrity CHECKs:**

| #   | CHECK                                                                                      | Catches                              |
| --- | ------------------------------------------------------------------------------------------ | ------------------------------------ |
| 1   | Exclusive arc: `(apps_id, category_id, domain_pattern)` exactly one non-null (or all null) | Policy on both an app and a category |
| 2   | `effect NOT IN (2,3) OR (time_limit_minutes > 0)`                                          | TimeLimit with no limit              |
| 3   | `effect NOT IN (0,1) OR time_limit_minutes IS NULL`                                        | Block with a time limit              |
| 4   | `json_type(schedule_json) IS 'array'`                                                      | Corrupted schedule data              |

These CHECKs mirror the domain type invariants exactly — a row read from this
table can construct a valid `Policy` struct with zero additional validation.

### `categories` — User-Defined Groupings

This table holds the category roster with a unique name constraint and optional
display metadata such as color and icon. Built-in categories are seeded at first
run.

### `app_categories` — App-to-Category Mappings

This table is the single source of truth for app categorization. Every row is
authoritative, whether seeded as a default or edited by the user. References
`apps.id` via `apps_id` FK.

| Column      | Type    | Notes                                             |
| ----------- | ------- | ------------------------------------------------- |
| apps_id     | INTEGER | FK to `apps.id`                                   |
| user_id     | INTEGER | 0 = system-global default, N = per-user override  |
| category_id | INTEGER | FK to `categories.id`; NULL = fall through to AI  |
| icon_path   | TEXT    | Optional per-user icon override                   |
| ignore      | INTEGER | When 1, app is excluded from tracking and reports |
| updated_at  | TEXT    | Last modification timestamp                       |

Resolution chain: user-specific row → system-global default (user_id=0) → AI
classification → Uncategorized.

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
lifts an active block on PolicyMutated; the GUI dashboard invalidates its
in-memory cache on any notification so the next render frame re-fetches.

---

## Query Patterns

### Daily Usage for Policy Evaluation

The policy engine reads daily_usage by date and app_id to obtain the total
minutes. This is a point lookup on the materialized table. The calling code
constructs the appropriate domain type from the result depending on whether the
policy is TimeLimit or Notify.

### Daily Usage Report for Dashboard

The dashboard reads total minutes per app for a given date by scanning
daily_usage filtered by date. The result is an ordered list of app totals.

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

When the system is about to suspend, hibernate, shut down, or log out, the open
focus interval must be closed so wall-clock time during the power state change
is not counted. A PowerStateWatcher subscribes to systemd-logind PrepareForSleep
and PrepareForShutdown signals via D-Bus. On PrepareForSleep(TRUE) it emits a
real Slept event; on PrepareForShutdown(TRUE) it emits a real ShutDown event.
These are genuine occurrences, so the event log stays truthful and the interval
is simply closed by the existing accumulation logic.

Session lifecycle events such as Locked and LoggedOut are emitted by the same
watcher from logind Session Lock and session-removed signals.

The daemon creates a **logind delay inhibitor**
(`inhibit("sleep:shutdown", "delay")`) for each sleep, shutdown, and logout
event. It then sends the close event to the enforcer actor and waits for the
flush acknowledgement before releasing the inhibitor. This guarantees that the
close event and its interval deltas are persisted to the database before the
power state change completes. If the flush fails, the error is logged and the
inhibitor is released anyway. Losing a few seconds of usage data is acceptable;
blocking a power state change indefinitely is not.

### Process Termination Handling

Logout does not trigger a logind PrepareForShutdown signal. The display manager
terminates the session compositor, which sends SIGHUP or SIGTERM to child
processes. A tokio signal handler hooks SIGTERM and SIGINT to emit a real
LoggedOut event before the process exits. This covers logout, Ctrl+C in a
terminal, systemctl --user stop, and terminal close. The handler accesses the
actor focus state and user_id needed to close the open interval, then cancels
the tokio runtime to stop all actors cleanly.
