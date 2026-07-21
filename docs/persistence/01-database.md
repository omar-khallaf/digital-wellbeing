# Database Design

## Stack

- **diesel** + **diesel-async** with SQLite backend.
- WAL mode: `PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;` — enables
  concurrent reads without blocking on writes.
- One connection per process, behind a connection pool (r2d2/bb8 via
  diesel-async).

---

## Store Module (`store/`)

The `store/` module owns the connection lifecycle and migration runner. Each
feature's `data/` module imports the pool type to execute queries.

```rust
// store/connection.rs

/// Thread-safe connection pool. The only way feature data modules obtain a
/// database connection. Constructed once at startup via `StoreBuilder`.
#[derive(Clone)]
pub struct DbPool {
    inner: Pool<AsyncSqliteConnection>, // diesel-async r2d2 pool
}

impl DbPool {
    /// Get a connection from the pool. Blocking waits are spawn_blocking'd.
    pub async fn get(&self) -> Result<AsyncSqliteConnection>;
}

/// Builder: the only way to obtain a DbPool. Ensures migrations run on the
/// pool before any connection is handed out.
pub struct StoreBuilder {
    db_path: PathBuf,
    pool_size: usize,
    wal_mode: bool,
}

impl StoreBuilder {
    pub fn new(db_path: PathBuf) -> Self;
    pub fn pool_size(mut self, n: usize) -> Self;
    /// Connect, run pending migrations, return ready-to-use pool.
    pub async fn build(self) -> Result<DbPool>;
}
```

### Migration Runner

Migrations live in `store/migrations.rs`. Diesel's embedded migrations are used
(SQL files under `migrations/` directory):

```rust
// store/migrations.rs
use diesel::sqlite::SqliteConnection;
use diesel_migrations::{embed_migrations, EmbeddedMigration, MigrationHarness};

pub const MIGRATIONS: EmbeddedMigration = embed_migrations!("migrations/");

/// Run all pending migrations. Called once at startup from StoreBuilder::build().
pub fn run_migrations(conn: &mut SqliteConnection) -> Result<()> {
    conn.run_pending_migrations(MIGRATIONS)
        .map(|_| ())?;
    Ok(())
}
```

### Connection Lifecycle

```text
Startup:
  StoreBuilder::new(db_path).build()
    → Open SQLite connection (WAL mode)
    → Run pending migrations
    → Return DbPool (backed by r2d2 pool)

Application lifetime:
  Feature actors call pool.get() to write events and read policy state.
  The gpui dashboard calls pool.get() to read daily usage, policies,
  and event history. WAL mode permits concurrent reads from
  the dashboard render loop and writes from feature actors.

Shutdown:
  On CancellationToken, actors flush a real close event (Slept/LoggedOut) to
  close open intervals (TrackerActor accumulates the active interval, then
  inserts the real event in a transaction), then the pool is dropped (all
  connections drained, WAL checkpointed).
```

### Dependency Rules

- `store/` depends on diesel, diesel-async, and the migration files.
- Feature `data/` modules depend on `store/` (for `DbPool`), never on diesel
  directly.
- Migration files live in `migrations/` at the project root — settled at build
  time by `embed_migrations!`.

---

## Schema

### `events` — Append-Only Event Log

Eight event types (see `EventType` enum). Every focus switch or state change
writes exactly one row.

**Design rationale — generated columns from JSON:** The `timestamp` and `app_id`
columns are **STORED generated columns** extracted from the JSON `payload` via
`json_extract()`. This gives us:

- **Single source of truth** — all field data is in the JSON payload; generated
  columns are derived and stay consistent.
- **Indexable** — STORED means the extracted values physically exist in the row,
  so indexes on `timestamp` and `app_id` work without function-wrapped lookups.
- **In-app maintainable** — interval accumulation is done in Rust via
  `accumulate_interval()`, called in the same transaction as the event INSERT.
  This keeps business logic in application code where it's debuggable and
  testable.
- **Normalized reads** — query code reads `timestamp` / `app_id` directly
  instead of calling `json_extract()` on every query.

**Storage optimization — short JSON keys:** Payload field names are shortened to
one character (`t` for timestamp, `a` for app_id) to reduce per-row JSON storage
overhead. At ~100K events/year, `"timestamp"` → `"t"` saves ~9 bytes and
`"app_id"` → `"a"` saves ~7 bytes per row, totalling ~1.6 MB/year saved on JSON
key names alone.

| Full key      | Short key | Bytes saved/row |
| ------------- | --------- | --------------- |
| `"timestamp"` | `"t"`     | 9               |
| `"app_id"`    | `"a"`     | 7               |

```sql
CREATE TABLE events (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    event_type  INTEGER NOT NULL CHECK(event_type IN (0, 1, 2, 3, 4, 5, 6, 7)),
    payload     TEXT NOT NULL,  -- JSON; short-key format: {"t":"...","a":"..."}
    user_id     INTEGER NOT NULL,  -- uid of the user who generated this event

    -- STORED generated columns: materialized from payload JSON at insert time.
    -- Queryable without json_extract() for index usage and read simplicity.
    timestamp   TEXT GENERATED ALWAYS AS (json_extract(payload, '$.t')) STORED NOT NULL,
    app_id      TEXT GENERATED ALWAYS AS (json_extract(payload, '$.a')) STORED,

    -- JSON schema validation per event type (see payload table below)
    -- NOTE: Use IS not = for json_type() because a missing key returns NULL,
    -- and NULL = 'text' evaluates to NULL (passes CHECK).  IS treats NULL
    -- as a value, so NULL IS 'text' = 0 (fails CHECK) as intended.
    CHECK (
        (event_type = 0
            AND json_type(payload) IS 'object'
            AND json_type(payload, '$.t') IS 'text'
            AND json_type(payload, '$.a') IS 'text')
        OR
        (event_type IN (1, 2, 3, 4, 5, 6, 7)
            AND json_type(payload) IS 'object'
            AND json_type(payload, '$.t') IS 'text'
            AND json_extract(payload, '$.a') IS NULL)
    )
);

CREATE INDEX idx_events_ts ON events(timestamp);
CREATE INDEX idx_events_app_ts ON events(app_id, timestamp) WHERE app_id IS NOT NULL;
CREATE INDEX idx_events_user_id ON events(user_id, id);  -- covering index: per-user queries filter by user_id and order by id
```

**EventType enum (Rust, serialized as integer discriminant):**

```rust
#[repr(u8)]
pub enum EventType {
    WindowFocused = 0, // App gained focus — app_id set, interval opens
    Unfocused     = 1, // No window focused (desktop/overview) — interval closes
    Idle          = 2, // User inactive (plugin idle signal) — interval pauses
    Resumed       = 3, // Activity resumed — interval unpauses
    Locked        = 4, // Screen locked — interval closes
    LoggedOut     = 5, // Session ended — interval closes
    Slept         = 6, // Suspend or hibernate — interval closes
    ShutDown      = 7, // Power off / reboot — interval closes
}
```

**Payload per type:**

| EventType     | Payload JSON                           | app_id (generated) | Meaning                              |
| ------------- | -------------------------------------- | ------------------ | ------------------------------------ |
| WindowFocused | `{"t":"2026-07-08 10:30:00","a":"fx"}` | `"fx"`             | App gained focus — interval opens    |
| Unfocused     | `{"t":"2026-07-08 10:30:05"}`          | `NULL`             | No window focused — interval closes  |
| Idle          | `{"t":"2026-07-08 10:15:00"}`          | `NULL`             | User inactive — interval pauses      |
| Resumed       | `{"t":"2026-07-08 10:22:00"}`          | `NULL`             | Activity resumed — interval unpauses |
| Locked        | `{"t":"2026-07-08 11:00:00"}`          | `NULL`             | Screen locked — interval closes      |
| LoggedOut     | `{"t":"2026-07-08 17:00:00"}`          | `NULL`             | Session ended — interval closes      |
| Slept         | `{"t":"2026-07-08 22:00:00"}`          | `NULL`             | Suspend/hibernate — interval closes  |
| ShutDown      | `{"t":"2026-07-08 23:00:00"}`          | `NULL`             | Power off/reboot — interval closes   |

The CHECK constraint enforces these rules at the DB level:

- `event_type` is `0` (WindowFocused) → payload must be an object with string
  `t` and string `a` fields.
- `event_type` is `1` (Unfocused) → payload must be an object with string `t`
  field; `a` must be absent or JSON null.
- No other event_type values are accepted.

**Interval computation (always at query time):**

Tracked time for an app = sum of the **active** (non-idle)
`diff(event[n], event[n+1])` segments where `event[n]` is
`WindowFocused{app_id=X}` and the interval is open:

- `WindowFocused` → next `WindowFocused`/close event: the whole span is active
  time for the first app (switched directly, no pause in between).
- `WindowFocused` → `Idle`: the span up to `Idle` is active. The `Idle` →
  `Resumed` span is **idle** and is NOT counted. `Resumed` → next event resumes
  active counting.
- `Idle`/`Resumed` carry **no** `app_id`; the app they pause is the open
  interval from the most recent `WindowFocused`.
- `Unfocused`, `Locked`, `LoggedOut`, `Slept`, `ShutDown` CLOSE the interval —
  the span from the last `WindowFocused` to the close event (minus any enclosed
  idle) is the final active time for that app. The span after a close event
  belongs to no app.

**Open intervals:** If the last event is `WindowFocused`, `Idle`, or `Resumed`
(no subsequent close event), the interval is still open. The current time is the
implicit end. Active time excludes any currently-open `Idle` pause
(`paused_at`). The in-memory `FocusState` tracks `paused_at`/`paused_total` (see
below) so this is computed without a full log scan per query.

**Why `AUTOINCREMENT`:** The `id` is used as an ordering token for the reactive
watch channel — consumers can track "last seen event id" to avoid re-processing
known events.

**Timestamp format:** All timestamps are stored as **`YYYY-MM-DD HH:MM:SS` in
UTC** (space-separated, no timezone offset), e.g. `2026-07-07 14:30:00`. This
single format satisfies two requirements at once:

- **Lexicographic ordering == chronological ordering** — all values are UTC, so
  `a < b` as strings iff `a` is earlier.
- **SQLite date functions parse it** — `strftime('%s', ...)` / `julianday(...)`
  accept the space-separated form, making query-time duration math
  straightforward.

Code formats timestamps with `dt.format("%Y-%m-%d %H:%M:%S").to_string()`
(chrono, UTC).

---

### `daily_usage` — Materialized Daily Usage Per App

Maintained by application-level transactions on `events` INSERT. Each event
INSERT is wrapped in a BEGIN/COMMIT pair that also calls `accumulate_interval()`
to update the materialized view.

```sql
CREATE TABLE daily_usage (
    date           TEXT NOT NULL,  -- ISO date (substr(timestamp, 1, 10))
    user_id        INTEGER NOT NULL,  -- uid for per-user scoping
    app_id         TEXT NOT NULL,
    total_seconds  INTEGER NOT NULL DEFAULT 0 CHECK(total_seconds >= 0),
    extended       INTEGER NOT NULL DEFAULT 0 CHECK(extended IN (0, 1)),  -- 1 = user has extended time today
    updated_at     TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    PRIMARY KEY (date, user_id, app_id)
);
```

#### Interval Accumulation (Application Level)

Instead of SQL triggers, interval accumulation is handled by a Rust function
`accumulate_interval()` called in the same explicit `BEGIN`/`COMMIT` transaction
as the event `INSERT`. This keeps all business logic in application code where
it's debuggable and testable.

**Focus state** is maintained in-memory by the `TrackerActor` as a
`HashMap<Uid, FocusState>`, not in a database table:

```rust
/// In-memory open-interval tracker per user. Lives in `TrackerActor`'s
/// `HashMap<Uid, FocusState>` — never persisted (the events log is the source
/// of truth; this is just the live accumulator).
struct FocusState {
    app_id: AppId,
    started_at: DateTime<Utc>,
    /// Set while an `Idle` pause is active; `None` = interval is counting.
    /// `Idle` is the ONLY pauser — suspend/lock/logout/shutdown CLOSE instead.
    paused_at: Option<DateTime<Utc>>,
    /// Idle time accumulated across all prior Idle/Resumed cycles.
    paused_total: Duration,
}

impl FocusState {
    /// Active (non-idle) seconds elapsed so far. Excludes all idle pauses.
    fn active_duration(&self, now: &DateTime<Utc>) -> i64 {
        let gross = (*now - self.started_at).num_seconds().max(0);
        let idle = self.paused_total.num_seconds()
            + self.paused_at
                .map(|p| (*now - p).num_seconds().max(0))
                .unwrap_or(0);
        (gross - idle).max(0)
    }

    fn is_paused(&self) -> bool {
        self.paused_at.is_some()
    }
}
```

**Accumulation transaction pattern** — when an event arrives, the write is
wrapped in an explicit transaction that also updates the materialized view:

```rust
/// Accumulate the prior focus interval into daily_usage.
/// Called inside the same transaction as the event INSERT.
fn accumulate_interval(
    conn: &mut AsyncSqliteConnection,
    user_id: Uid,
    prev_focus: &FocusState,
    now: &DateTime<Utc>,
) -> QueryResult<()> {
    let duration = prev_focus.active_duration(now);
    if duration <= 0 {
        return Ok(()); // No measurable active time (entirely idle)
    }

    let date = prev_focus.started_at.format("%Y-%m-%d").to_string();

    // UPSERT into daily_usage within the same transaction
    diesel::insert_into(daily_usage::table)
        .values(&DailyUsageUpsert {
            date: &date,
            user_id: user_id.as_ref(),
            app_id: prev_focus.app_id.as_ref(),
            total_seconds: duration as i32,
        })
        .on_conflict((daily_usage::date, daily_usage::user_id, daily_usage::app_id))
        .do_update()
        .set(daily_usage::total_seconds.eq(
            daily_usage::total_seconds + duration as i32,
        ))
        .execute(conn)?;

    Ok(())
}

/// Insert a new event, accumulating the prior interval in the same transaction.
pub async fn insert_event(
    conn: &mut AsyncSqliteConnection,
    user_id: Uid,
    new_event: NewEvent,
    focus_state: &mut HashMap<Uid, FocusState>,
) -> QueryResult<()> {
    conn.transaction(|conn| {
        let now = Utc::now();

        // 1. Accumulate the previous interval (if any)
        if let Some(prev) = focus_state.get(&user_id) {
            accumulate_interval(conn, user_id, prev, &now)?;
        }

        // 2. Insert the new event
        diesel::insert_into(events::table)
            .values(&new_event)
            .execute(conn)?;

        // 3. Update in-memory focus state
        match new_event.event_type {
            // Close events: credit prior active interval, then clear state.
            1 | 4 | 5 | 6 | 7 => {
                if let Some(prev) = focus_state.get(&user_id) {
                    accumulate_interval(conn, user_id, prev, &now)?;
                }
                diesel::insert_into(events::table)
                    .values(&new_event).execute(conn)?;
                focus_state.remove(&user_id);
            }
            // WindowFocused: credit prior (if any), then open a new interval.
            0 => {
                if let Some(prev) = focus_state.get(&user_id) {
                    accumulate_interval(conn, user_id, prev, &now)?;
                }
                diesel::insert_into(events::table)
                    .values(&new_event).execute(conn)?;
                focus_state.insert(user_id, FocusState {
                    app_id: new_event.app_id.clone(),
                    started_at: now,
                    paused_at: None,
                    paused_total: Duration::zero(),
                });
            }
            // Idle: pause (freeze timer). Ignore if already paused.
            2 => {
                diesel::insert_into(events::table)
                    .values(&new_event).execute(conn)?;
                if let Some(fs) = focus_state.get_mut(&user_id) {
                    if fs.paused_at.is_none() {
                        fs.paused_at = Some(now);
                    }
                }
            }
            // Resumed: unpause; fold the just-ended idle span into paused_total.
            3 => {
                diesel::insert_into(events::table)
                    .values(&new_event).execute(conn)?;
                if let Some(fs) = focus_state.get_mut(&user_id) {
                    if let Some(p) = fs.paused_at.take() {
                        fs.paused_total += *now - p;
                    }
                }
            }
            _ => {} // unknown event type — no state change
        }

        Ok(())
    }).await
}
```

The `conn.transaction()` call guarantees atomicity: if either the accumulation
or the event INSERT fails, both are rolled back and the in-memory focus state
remains consistent (it is updated only after the transaction commits).

##### Extended flag update

The `extended` flag is set when the policy engine grants an extension. It is not
set by a trigger (the policy engine makes the decision) — it is set via a direct
`UPDATE` by the EnforcerActor after granting extra time:

```sql
-- Called by EnforcerActor when user clicks "Extend" on a block overlay.
UPDATE daily_usage SET extended = 1, updated_at = ...
WHERE date = ? AND app_id = ?;
```

**Reading `extended` at policy evaluation time:**

```sql
SELECT total_seconds, extended FROM daily_usage
WHERE date = ? AND app_id = ?;
```

The policy engine constructs the domain types from this:

```rust
/// Unifies tracked state for both blocking and notify-only policies.
pub enum TrackedApp {
    /// Hard deadline with optional extension (TimeLimit/Block policies).
    TimeLimited(TimeLimitedApp),
    /// Tracked usage with notification reminders (Notify policies).
    TimeTracked(TimeTrackedApp),
}

impl TrackedApp {
    pub fn used(&self) -> Duration { /* delegates to inner */ }
    pub fn remaining(&self) -> Duration { /* delegates to inner */ }
}

/// TimeLimit policy state — hard limit with optional user extension.
pub enum TimeLimitedApp {
    /// Normal regime: used within policy time_limit.
    Normal(used: i64, limit: i64),
    /// Extended regime: user already extended; limit is time_limit + extra.
    Extended(used: i64, limit: i64),
}

impl TimeLimitedApp {
    pub fn remaining(&self) -> i64 { ... }
    pub fn can_extend(&self) -> bool { matches!(self, Normal(..)) }
}

/// Notify policy state — simple struct, no state machine.
/// Notification scheduling is ephemeral (EnforcerActor timers), not persisted.
pub struct TimeTrackedApp {
    pub used: i64,
    pub limit: i64,
}

impl TimeTrackedApp {
    pub fn remaining(&self) -> i64 {
        (self.limit - self.used).max(0)
    }
}
```

Resolution — constructs the appropriate variant per policy kind:

```rust
fn app_state(usage: &DailyUsageRow, policy: &Policy) -> TrackedApp {
    match policy.kind {
        PolicyKind::Block => unreachable!("Block has no tracked state"),
        PolicyKind::TimeLimit => {
            let base = policy.time_limit_seconds.expect("time_limit requires limit");
            let app = if usage.extended {
                TimeLimitedApp::Extended(usage.total_seconds, base + policy.extra_seconds)
            } else {
                TimeLimitedApp::Normal(usage.total_seconds, base)
            };
            TrackedApp::TimeLimited(app)
        }
        PolicyKind::Notify => {
            TrackedApp::TimeTracked(TimeTrackedApp {
                used: usage.total_seconds,
                limit: policy.time_limit_seconds.expect("notify requires limit"),
            })
        }
    }
}
```

#### Why application-level transactions instead of triggers?

Application-level transactions provide the same atomicity guarantee —
`BEGIN`/`COMMIT` wraps both the event INSERT and the daily_usage UPSERT — while
keeping all business logic in Rust where it's testable and debuggable. If the
daemon crashes mid-transaction, the entire operation is rolled back, preserving
crash consistency. The materialized view still provides the same read simplicity
(point lookup on `daily_usage`).

---

### `policies` — Blocking, Time Limit & Notify Rules

```sql
CREATE TABLE policies (
    id          INTEGER PRIMARY KEY,
    name        TEXT NOT NULL,
    kind        INTEGER NOT NULL CHECK(kind IN (0, 1, 2)),  -- 0=Block, 1=TimeLimit, 2=Notify
    category_id INTEGER REFERENCES categories(id) ON DELETE CASCADE,
    app_id      TEXT,
    time_limit_seconds            INTEGER,
    extra_seconds                 INTEGER NOT NULL DEFAULT 600 CHECK(extra_seconds >= 0),
    notification_repeat_interval_seconds INTEGER,  -- NULL=once, >0=repeat N sec
    schedule_json                 TEXT NOT NULL,  -- serialized TimeWindow rules
    active      INTEGER NOT NULL DEFAULT 1 CHECK(active IN (0, 1)),
    created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    updated_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    owner_id    INTEGER NOT NULL DEFAULT 0,  -- uid the policy applies to (RBAC scoping)
    created_by  INTEGER NOT NULL DEFAULT 0,  -- uid that created the policy (RBAC ownership)

    -- Exclusive arc: targets EITHER a category OR an app, never both, never neither
    CHECK (
        (category_id IS NOT NULL AND app_id IS NULL)
        OR (category_id IS NULL AND app_id IS NOT NULL)
    ),

    -- Kind-based: Block (0) has no time limit; TimeLimit (1) and Notify (2) require one
    CHECK (NOT (kind = 0 AND time_limit_seconds IS NOT NULL)),
    CHECK (NOT (kind IN (1, 2) AND time_limit_seconds IS NULL)),

    -- time_limit_seconds must be positive when set (0-limit is a Block, not TimeLimit/Notify)
    CHECK (time_limit_seconds IS NULL OR time_limit_seconds > 0),

    -- notification_repeat_interval_seconds is only valid for Notify (kind=2) and must be >0
    CHECK (notification_repeat_interval_seconds IS NULL
        OR (kind = 2 AND notification_repeat_interval_seconds > 0)),

    -- schedule_json must be a valid JSON object
    CHECK (json_type(schedule_json) IS 'object')
);

CREATE INDEX idx_policies_active ON policies(active) WHERE active = 1;
CREATE INDEX idx_policies_owner ON policies(owner_id);
```

`extra_seconds` configures how much extra time the user gets when they click
"Extend" on a block overlay for this policy. When extended, the effective limit
becomes `time_limit_seconds + extra_seconds`.

`notification_repeat_interval_seconds` controls how often to re-notify the user
after the limit is exceeded (Notify kind only). NULL = notify once.

**Design:** Policies target either a category (FK with cascade) or a specific
app, never both. The exclusive arc CHECK prevents orphan targeting.

| kind | name      | time_limit_seconds       | extra_seconds | notification_repeat_interval_seconds |
| ---- | --------- | ------------------------ | ------------- | ------------------------------------ |
| 0    | Block     | NULL                     | >= 0 (unused) | NULL (unused)                        |
| 1    | TimeLimit | > 0 (required)           | >= 0          | NULL (unused)                        |
| 2    | Notify    | > 0 (required, was NULL) | >= 0 (unused) | NULL = once, > 0 = repeat            |

**CHECK constraints enforce domain invariants:**

- `time_limit_seconds`: NULL for Block, > 0 for TimeLimit/Notify (0-limit is
  semantically a Block)
- `extra_seconds >= 0`: extension duration cannot be negative
- `notification_repeat_interval_seconds`: only Notify (kind=2) can set this;
  when set, must be > 0
- `schedule_json`: validated as a JSON object at the DB level
- `active IN (0, 1)`: boolean flag

**Why not `target_type` + `target_id` text field?** Polymorphic associations
prevent FK constraints. If a category is renamed, policies with
`target_type='category'` referencing its name silently break. With a real FK,
`ON DELETE CASCADE` handles it.

**Why integer `kind`?** Compactness — integer bytes vs. text string per row. The
`PolicyKind` Rust enum maps 1:1 with integer discriminants:

```rust
#[repr(u8)]
pub enum PolicyKind {
    Block = 0,      // Unconditional block, no time tracking
    TimeLimit = 1,  // Block when daily limit exceeded
    Notify = 2,     // Notify via D-Bus (freedesktop) when daily limit exceeded
}
```

---

### `categories` — User-Defined Groupings

```sql
CREATE TABLE categories (
    id          INTEGER PRIMARY KEY,
    name        TEXT NOT NULL UNIQUE,
    color       TEXT,   -- hex color e.g. '#FF6B6B'
    icon        TEXT,   -- icon name or path
    created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);
```

Built-in categories seeded at first run: Productivity, Communication,
Entertainment, Social, Development, Utilities, Uncategorized.

---

### `app_categories` — App-to-Category Mappings

**Single source of truth** for app categorization. Replaces both the former
`app_overrides` table and the `app-categories.toml` heuristic data file.
Built-in defaults are seeded at migration time; user modifications overwrite the
same rows — there is no separate "override" concept.

```sql
CREATE TABLE app_categories (
    app_id          TEXT NOT NULL,
    user_id         INTEGER NOT NULL DEFAULT 0,
    category_id     INTEGER REFERENCES categories(id) ON DELETE SET NULL,
    display_name    TEXT,       -- overrides raw app_id for UI display
    icon_path       TEXT,       -- overrides default icon
    ignore          INTEGER NOT NULL DEFAULT 0 CHECK(ignore IN (0, 1)),
    updated_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    PRIMARY KEY (app_id, user_id)
);
```

**Design:**

- Every row is authoritative — whether seeded as a default or set by the user.
- `user_id=0` represents system-global defaults (seeded at migration).
- Per-user overrides use `user_id=N` (the caller's UID from D-Bus
  `SO_PEERCRED`).
- Resolution: check `user_id=N` first; if no row exists, fall back to
  `user_id=0`.
- `category_id` is nullable: when NULL the categorizer falls through to AI
  classification → `Uncategorized`.
- No FK to `events` because an `app_id` may appear in events before any
  `app_categories` entry exists, and deleting an entry must not cascade.
- Seeded defaults use `INSERT OR IGNORE` so future migration additions never
  overwrite user edits.

**Resolution priority:**

```
app_categories (user-specific) → app_categories (system-global, user_id=0) → AI classification → Uncategorized
```

Query pattern:
`LEFT JOIN app_categories ON events.app_id = app_categories.app_id AND app_categories.user_id = ?`

---

## Reactive System

Events form the reactive data surface. When a new event is written, consumers
must re-evaluate their state (timers, UI aggregates, block state).

A tagged notification enum lets consumers skip irrelevant work:

```rust
/// Tagged notification — consumers filter by variant to skip
/// unnecessary re-evaluation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReactiveNotification {
    /// New events written (focus switch, Unfocused).
    EventWritten,
    /// Policy or category mutated (limits, schedule, active flag).
    PolicyMutated,
}

/// Coalescing dirty-flag (backed by a `watch` channel), **not** an event bus.
/// It carries only the *latest* variant, so concurrent `EventWritten` +
/// `PolicyMutated` notifications coalesce to the last one. Consumers treat any
/// notification as "state may have changed — re-check / invalidate cache", which
/// is exactly what the actors and dashboard need.
#[derive(Clone)]
pub struct ReactiveNotifier {
    inner: Arc<watch::Sender<ReactiveNotification>>,
}

impl ReactiveNotifier {
    pub fn new() -> (Self, watch::Receiver<ReactiveNotification>) {
        let (tx, rx) = watch::channel(ReactiveNotification::EventWritten);
        (Self { inner: Arc::new(tx) }, rx)
    }

    pub fn notify_event_written(&self) {
        let _ = self.inner.send(ReactiveNotification::EventWritten);
    }

    pub fn notify_policy_mutated(&self) {
        let _ = self.inner.send(ReactiveNotification::PolicyMutated);
    }
}
```

**Notification triggers:**

| Mutation                              | Notification variant | Why                                                     |
| ------------------------------------- | -------------------- | ------------------------------------------------------- |
| `events` INSERT                       | `EventWritten`       | Timer recalculation, UI refresh                         |
| `policies` INSERT/UPDATE/DELETE       | `PolicyMutated`      | Timer recalculation, UI refresh                         |
| `categories` INSERT/UPDATE/DELETE     | `PolicyMutated`      | Affects policy target resolution                        |
| `app_categories` INSERT/UPDATE/DELETE | `PolicyMutated`      | Changes categorization — policy evaluation may re-check |

The tracker, enforcer, and dashboard each hold a
`watch::Receiver<ReactiveNotification>` cloned from the notifier. On
notification, they check the variant:

- **Tracker** — on `EventWritten`, updates active window state (the latest event
  determines the currently focused app). On `PolicyMutated`, re-evaluates
  whether the current app is approaching a new limit.
- **Enforcer** — on `EventWritten`, re-evaluates blocks. On `PolicyMutated`,
  checks if a policy change lifts an active block.
- **Dashboard** — on any notification, invalidates the 100ms
  stale-while-revalidate cache for the next render frame.

The notifier is wired in `main.rs` after `StoreBuilder::build()` and passed to
each actor that needs it.

---

## Query Patterns

### Daily Usage (for Policy Evaluation)

```rust
/// Returns today's usage + extended flag for a specific app.
/// Point lookup on materialized table.
/// The calling code constructs the appropriate domain type (TrackedApp)
/// based on the PolicyKind — TimeLimit policies read `extended`,
/// Notify policies ignore it.
pub fn todays_usage_for_app(
    app_id: &AppId,
    today: NaiveDate,
    conn: &mut AsyncSqliteConnection,
) -> QueryResult<Option<(i64, bool)>> {
    daily_usage::table
        .filter(daily_usage::date.eq(today.to_string()))
        .filter(daily_usage::app_id.eq(app_id.as_ref()))
        .select((daily_usage::total_seconds, daily_usage::extended))
        .first(conn)
        .await
        .optional()
}
```

### Daily Usage Report (Dashboard)

```rust
/// Returns total seconds + extended flag per app for today.
/// O(n) scan over the materialized table.
pub fn daily_usage(today: NaiveDate, conn: &mut AsyncSqliteConnection) -> QueryResult<Vec<(String, i64, bool)>> {
    daily_usage::table
        .filter(daily_usage::date.eq(today.to_string()))
        .select((daily_usage::app_id, daily_usage::total_seconds, daily_usage::extended))
        .load(conn)
        .await
}
```

### Last Event (Boot Reconciliation)

```rust
/// Returns the most recent event, if any.
/// Used at startup to reconcile with plugin state.
pub fn last_event(conn: &mut AsyncSqliteConnection) -> QueryResult<Option<EventRow>> {
    events::table
        .order(events::id.desc())
        .first(conn)
        .await
        .optional()
}
```

### Historical Report (Raw Events)

For infrequent OLAP-style queries:

```rust
/// Returns all events within a date range for report generation.
pub fn event_range(
    range_start: NaiveDate,
    range_end: NaiveDate,
    conn: &mut AsyncSqliteConnection,
) -> QueryResult<Vec<EventRow>> {
    events::table
        .filter(events::timestamp.ge(range_start.to_string()))
        .filter(events::timestamp.lt(range_end.to_string()))
        .order(events::timestamp.asc())
        .load(conn)
        .await
}
```

### Open Interval at Query Time

The currently focused app is tracked in-memory by the `TrackerActor`
(`HashMap<Uid, FocusState>`). The dashboard and policy engine query the actor's
state directly — no DB query needed:

```rust
/// Returns the currently open focus interval, if any, from actor state.
/// Called by the dashboard and policy engine each render frame /
/// policy evaluation cycle.
pub fn open_interval_from_actor(
    focus_state: &HashMap<Uid, FocusState>,
    uid: &Uid,
) -> Option<(AppId, DateTime<Utc>)> {
    focus_state.get(uid).map(|fs| (fs.app_id.clone(), fs.started_at))
}
```

For historical consistency checks at startup (after process restart), the last
event in the `events` table is used — if the most recent event is
`WindowFocused`, the interval is still open:

```rust
/// Startup reconciliation: reconstruct any open interval from the tail of the
/// log. The interval is OPEN iff the last event is NOT a close event
/// (Unfocused/Locked/LoggedOut/Slept/ShutDown). The app_id comes from the most
/// recent WindowFocused; a tail `Idle` means the interval is still paused.
/// (The logind delay inhibitor flushes a real Slept/ShutDown before power-off,
/// so a dangling `Idle` at startup is exceptional — it is treated as open +
/// paused and resolved by the next real event.)
pub fn open_interval_at_startup(
    conn: &mut AsyncSqliteConnection,
) -> QueryResult<Option<(String, NaiveDateTime, bool)>> {
    let last = last_event(conn).await?;

    match last {
        // Close events → interval already closed.
        Some(e) if matches!(e.event_type, 1 | 4 | 5 | 6 | 7) => Ok(None),
        // WindowFocused / Idle / Resumed → interval open.
        Some(e) => {
            let paused = e.event_type == 2; // tail Idle ⇒ still paused
            // app_id: from this event if WindowFocused, else from the most
            // recent WindowFocused (Idle/Resumed carry no app_id).
            let app_id = if e.event_type == 0 {
                e.app_id.clone().unwrap_or_default()
            } else {
                last_window_focused_app_id(conn).await?.unwrap_or_default()
            };
            let started = NaiveDateTime::parse_from_str(&e.timestamp, "%Y-%m-%d %H:%M:%S").ok();
            Ok(started.map(|ts| (app_id, ts, paused)))
        }
        _ => Ok(None),
    }
}
```

---

## Batch Write Strategy

Events are written on demand — each window focus switch or `Unfocused` produces
exactly one row. Write frequency is bounded by user interaction rate (max ~1
event per 100ms during rapid alt-tabbing). This is well within SQLite's
single-insert throughput for a WAL-mode database.

### Background Prune

Data retention is handled by an explicit async background task.

```rust
/// Raw event log retention — supports audit + historical reports.
const EVENTS_RETENTION_DAYS: i64 = 90;
/// Materialized daily aggregates retention — supports monthly reports
/// and usage-trend visualization. Kept longer than events because the
/// table is tiny (PK on date+app_id).
const DAILY_USAGE_RETENTION_DAYS: i64 = 90;

pub async fn prune_loop(mut conn: AsyncSqliteConnection) -> Result<()> {
    let mut interval = tokio::time::interval(Duration::from_secs(3600));
    loop {
        interval.tick().await;
        let events_cutoff = Utc::now() - Duration::days(EVENTS_RETENTION_DAYS);
        let usage_cutoff = (Utc::now() - Duration::days(DAILY_USAGE_RETENTION_DAYS))
            .format("%Y-%m-%d").to_string();

        // Prune raw events — batched LIMIT 500 to avoid WAL bloat / long locks.
        loop {
            let deleted = diesel::delete(
                events::table
                    .filter(events::timestamp.lt(events_cutoff.format("%Y-%m-%d %H:%M:%S").to_string()))
                    .limit(500)
            ).execute(&mut conn).await?;
            if deleted == 0 { break; }
        }

        // Prune daily_usage — date column is "YYYY-MM-DD" text, which sorts
        // lexicographically.
        loop {
            let deleted = diesel::delete(
                daily_usage::table
                    .filter(daily_usage::date.lt(usage_cutoff.clone()))
                    .limit(500)
            ).execute(&mut conn).await?;
            if deleted == 0 { break; }
        }
    }
}
```

The inner `loop { LIMIT 500 }` pattern prevents WAL file bloat and long table
locks that a single unbounded DELETE would cause on a large events table.

### Power State-Aware Flush

When the system is about to suspend, hibernate, or shut down, the open focus
interval must be closed so wall-clock time during the power state change is not
counted against the app limit. The `PowerStateWatcher` subscribes to
systemd-logind `PrepareForSleep` and `PrepareForShutdown` signals via D-Bus and
emits **real** events — `Slept` (suspend/hibernate) and `ShutDown` (power
off/reboot) — never a synthetic `Unfocused`. These are genuine occurrences, so
the event log stays truthful and the interval is simply closed:

- `PrepareForSleep(TRUE)` → `Slept` (covers both suspend and hibernate — logind
  cannot distinguish them at signal time)
- `PrepareForShutdown(TRUE)` → `ShutDown`

Session lifecycle (`Locked`, `LoggedOut`) is emitted by the same watcher from
logind `Session` `Lock` / session-removed signals (see
`../architecture/03-linux-platform.md`).

```rust
/// Close any open interval by emitting a REAL close event
/// (`Slept`/`ShutDown`/`Locked`/`LoggedOut`). Used by `PowerStateWatcher` on
/// power/session changes. Never emits a synthetic `Unfocused` — the event is
/// the genuine occurrence, so the log stays truthful.
pub async fn flush_close_event(
    conn: &mut AsyncSqliteConnection,
    focus_state: &HashMap<Uid, FocusState>,
    user_id: Uid,
    event_type: EventType,
) -> Result<()> {
    // Interval is open iff present in the in-memory map.
    if !focus_state.contains_key(&user_id) {
        return Ok(());
    }

    let now = Utc::now();

    // Insert the real close event in a transaction that also accumulates the
    // active interval. focus_state is updated after the transaction commits.
    conn.transaction(|conn| {
        if let Some(prev) = focus_state.get(&user_id) {
            accumulate_interval(conn, user_id, prev, &now)?;
        }

        diesel::insert_into(events::table)
            .values(&NewEvent {
                event_type: event_type as i32,
                payload: serde_json::json!({"t": now.format("%Y-%m-%d %H:%M:%S").to_string()}),
            })
            .execute(conn)?;

        Ok(())
    }).await?;

    focus_state.remove(&user_id);
    Ok(())
}
```

This prevents a suspend at 14:00 followed by resume at 16:00 from attributing 2
hours of "active" usage to the last focused app — the `Slept` event at 14:00
closes the interval before the system pauses.

**Error handling:** If the flush fails, log the error and release the D-Bus
delay inhibitor anyway. Losing a few seconds of usage data is acceptable;
blocking a power state change is not.

### Process Termination Handling (SIGTERM/SIGHUP)

Logout does **not** trigger a logind `PrepareForShutdown` signal. The display
manager terminates the session compositor (Hyprland/KWin), which sends
SIGHUP/SIGTERM to child processes. Without a handler, open intervals would
accrue wall-clock time until the next daemon restart.

A tokio signal handler hooks `SIGTERM` and `SIGINT` to emit a real `LoggedOut`
event (closing the interval) before exit:

```rust
use tokio::signal::unix::{signal, SignalKind};

/// Spawn a background task that inserts a real `LoggedOut` event on SIGTERM/SIGINT.
/// Used to handle logout (no logind signal) and Ctrl+C in terminal.
pub async fn spawn_termination_handler(pool: DbPool, cancel: CancellationToken) {
    let mut term = signal(SignalKind::terminate()).expect("SIGTERM handler");
    let mut intr = signal(SignalKind::interrupt()).expect("SIGINT handler");

    tokio::select! {
        _ = term.recv() => {}
        _ = intr.recv() => {}
    }

    if let Ok(mut conn) = pool.get().await {
        // Emit a real LoggedOut event — closes the open interval. Never a
        // synthetic event; the session genuinely ended. The handler must
        // close over the actor's `focus_state` and `user_id`.
        flush_close_event(&mut conn, &focus_state, user_id, EventType::LoggedOut).await.ok();
    }

    cancel.cancel();  // Signal all actors to stop
}
```

This covers:

- **Logout** — session compositor exits → SIGHUP → SIGTERM to daemon
- **Ctrl+C** — SIGINT
- **systemctl --user stop digital-wellbeing** — SIGTERM
- **Terminal close** — SIGHUP → SIGTERM

---

## Migration Policy

Managed by `diesel migration` CLI. The policy is **forward-only and additive**.
Rollback is achieved by deploying the previous binary, not by running
`down.sql`.

```text
Principles:

  1. Additive only. Every migration adds new tables, new columns, or new
     indexes. Never drop or rename columns/tables. This ensures the old
     binary can still read rows written by the new binary.
  2. New columns are nullable or have defaults. Old code must be able
     to read rows written by new code without schema awareness.
  3. Removal is a two-release process:
     Release N: Add new schema, keep old. Write both, read old.
     Release N+1: Migrate old → new. Drop old schema.
  4. No down.sql. We do not write down migrations. Rollback = deploy the
     previous binary. The additive policy guarantees compatibility.
  5. Failed migration = process exit. Migrations run at startup in
     StoreBuilder::build(). On restart, Diesel re-runs the failed migration
     (SQLite DDL is transactional as of 3.28.0 for most DDL statements).
```

```bash
diesel migration generate initial_schema
# edit up.sql only (no down.sql needed)
diesel migration run
```

The `store/migrations.rs` module runs pending migrations at startup via
`diesel_async::AsyncConnection::run_pending_migrations()`.
