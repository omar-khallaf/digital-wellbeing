# Testing Philosophy

## Tenet: Test Behavior, Not Structure

The type system enforces invariants (no raw `String`, no invalid states). Tests
exist to verify **business rules** and **domain logic**, not structural trivia.

### Bad (testing the compiler's job)

```rust
#[test]
fn app_id_rejects_empty() {
    assert!(AppId::new("".into()).is_err());
}
```

The `NonEmptyString` newtype already guarantees this at compile time for any
code path. Redundant.

### Good (testing the business rule)

```rust
#[test]
fn given_premium_member_and_100_item_then_10_percent_discount_applied() {
    // GIVEN
    let member = Membership::Premium;
    let cart = Cart::empty();

    // WHEN
    cart.add_item(Item::new(100_00));
    let total = cart.checkout(member);

    // THEN
    assert_eq!(total, 90_00); // 10% premium discount
}
```

---

## Pattern: Given-When-Then

Every test follows this structure explicitly:

```text
GIVEN [preconditions / system state]
WHEN  [action / command / event]
THEN  [expected outcome / domain events / state change]
```

```rust
#[test]
fn given_time_limit_policy_for_app_when_elapsed_exceeds_limit_then_verdict_is_block() {
    // GIVEN
    let policy = Policy::App {
        id: PolicyId::new(1),
        name: "Firefox limit".into(),
        config: PolicyConfig {
            kind: PolicyKind::TimeLimit,
            time_limit_seconds: Some(3600),
            extra_seconds: 600,
            schedule: TimeWindow::always(),
        },
        app_id: AppId::unchecked("firefox"),
    };
    let elapsed = Duration::minutes(90); // 5400s > 3600s limit
    let now = Utc.with_ymd_and_hms(2026, 7, 7, 23, 30, 0).unwrap();

    // WHEN
    let verdict = evaluate(
        &AppId::unchecked("firefox"),
        &[policy],
        elapsed,
        now,
    );

    // THEN
    assert!(matches!(verdict, PolicyVerdict::Block { .. }));
}
```

---

## Assert Both Domain Events and Database State

Domain events verify that business logic reached the right decision. Database
state verifies that the decision was persisted correctly. **Both are required**
— events are ephemeral (they exit the actor boundary), while the DB is the
canonical record for reports and crash recovery.

```rust
#[test]
fn tracking_app_focused_records_window_focused_event() {
    // GIVEN
    let mut tracker = TrackerState::new();
    let mut conn = MockDb::new().await;

    // WHEN
    let events = tracker.handle_event(PlatformEvent::WindowFocused {
        app_id: AppId::new("Alacritty").unwrap(),
        title: WindowTitle::new("zsh").unwrap(),
        pid: Pid::new(1234),
        uid: Uid::new(1000),
        overlay_shown: false,
    });

    // THEN — assert domain events (business logic decision)
    assert!(events.iter().any(|e| matches!(e, DomainEvent::WindowFocusChanged { .. })));

    // THEN — assert DB state (storage correctness)
    let db_events: Vec<(String, i32)> = conn.query_all(
        "SELECT app_id, event_type FROM events WHERE event_type = 0"
    ).await;
    assert_eq!(db_events.len(), 1);
    assert_eq!(db_events[0].0, "Alacritty");
    assert_eq!(db_events[0].1, 0); // WindowFocused
}
```

Assert both the decision path (events) and the persistence path (DB rows); a
test that only checks events can pass while the DB write silently fails (e.g., a
connection error that is logged but swallowed).

---

## Sociable Tests, Not Isolated Tests

**Do NOT mock** value objects, entities, or same-module collaborators. Let them
interact naturally. Only mock at system boundaries:

| Mock-worthy                              | Not worth mocking                 |
| ---------------------------------------- | --------------------------------- |
| Platform (via `MockPlatform`)            | `AppId`, `Duration`, `Policy`     |
| SQLite connection (in-memory via diesel) | `PolicyVerdict`, `WindowTitle`    |
| System clock (for time-dependent tests)  | `SessionSummary`, `TrackingState` |
| Compositor socket (internal Linux tests) | Classification rules              |

### MockPlatform Pattern

The primary test boundary is `Platform`. A concrete `MockPlatform` replays
recorded event traces and fakes all platform operations:

```rust
/// Mock platform. `from_events` shares the event deque between the returned
/// platform handle and the stream, so the actor consumes exactly the seeded
/// events. (A naive `init()` that rebuilt two empty instances would drop the
/// trace and deliver zero events — do not do that.)
/// show_overlay / hide_overlay are inlined directly — always succeed,
/// record the last config for assertions.
#[derive(Clone)]
struct MockPlatform {
    events: Arc<Mutex<VecDeque<PlatformEvent>>>,
    last_overlay_config: Arc<Mutex<Option<OverlayConfig>>>,
}

/// Stream half of the mock — owns a clone of the shared event deque.
struct MockEventStream {
    events: Arc<Mutex<VecDeque<PlatformEvent>>>,
}

impl Stream for MockEventStream {
    type Item = PlatformEvent;
    fn poll_next(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        // std Mutex is acceptable here: poll_next never blocks (pure pop).
        Poll::Ready(self.events.lock().unwrap().pop_front())
    }
}

impl Platform for MockPlatform {
    type EventStream = MockEventStream;

    async fn show_overlay(&self, config: OverlayConfig) -> Result<()> {
        *self.last_overlay_config.lock().unwrap() = Some(config);
        Ok(())
    }

    async fn hide_overlay(&self, _app_id: &AppId) -> Result<()> { Ok(()) }

    async fn notify(&self, _title: &str, _body: &str) -> Result<()> { Ok(()) }
}

impl MockPlatform {
    /// Construct with a pre-recorded event trace. The same `Arc<Mutex<..>>`
    /// deque is shared by the platform handle and the returned stream, so the
    /// actor receives exactly the events passed in (unlike a naive `init()`
    /// that would rebuild two empty instances and drop the trace).
    pub fn from_events(events: Vec<PlatformEvent>) -> (Self, impl Stream<Item = PlatformEvent>) {
        let deque = Arc::new(Mutex::new(VecDeque::from(events)));
        let platform = Self {
            events: deque.clone(),
            last_overlay_config: Arc::new(Mutex::new(None)),
        };
        (platform, MockEventStream { events: deque })
    }
}
```

All feature-level integration tests use `MockPlatform`. The Linux platform is
tested via a separate integration suite (for compositor-specific behavior, a
`MockCompositor` exists internal to `platform/linux/`).

---

## Test Organization

Each feature directory has its own tests:

```text
tracking/
├── domain/
│   └── mod.rs          // #[cfg(test)] mod tests { … }
├── core/
│   └── mod.rs          // #[cfg(test)] mod tests { … }
└── tests/              // Integration tests (cross-feature)
    └── tracking_test.rs
```

- **Unit tests** (`#[cfg(test)] mod tests` inside each module): Test domain
  logic, state machines, pure functions. Sociable within the module.
- **Integration tests** (`tests/`): Test feature boundaries, async actor
  behavior with `MockPlatform`, full pipeline from event to store.

---

## What NOT to Test

- Structural invariants guaranteed by the type system (`AppId` rejects empty)
- Simple field accessors / getters
- Third-party behavior (SQLite, gpui, tokio channels)
- Code that is "too simple to break" — classification overrides that are a
  `HashMap::get`, for example

Use judgment: if the test would test the compiler or the standard library,
delete it.

---

## Property-Based Testing for the Policy Engine

The `evaluate()` function is a pure function mapping
`(app_id, &[Policy], elapsed_usage, now) → PolicyVerdict` — an ideal candidate
for property-based testing with `proptest` or `bolero`.

### Invariants to verify

```rust
use proptest::prelude::*;

proptest! {
    #[test]
    fn all_ok_policies_produce_ok_verdict(
        app_id in app_id_strategy(),
        policies in vec(any_ok_policy(), 0..5),
        elapsed in 0i64..86400,  // 0–24h in seconds
        now in datetime_strategy(),
    ) {
        let verdict = evaluate(&app_id, &policies, Duration::seconds(elapsed), now);
        // If no policy has a limit, verdict must not be Block
        prop_assert!(!matches!(verdict, PolicyVerdict::Block { .. }));
    }

    #[test]
    fn at_most_one_block_verdict(
        app_id in app_id_strategy(),
        policies in vec(any_policy(), 1..10),
        elapsed in 0i64..86400,
        now in datetime_strategy(),
    ) {
        let verdict = evaluate(&app_id, &policies, Duration::seconds(elapsed), now);
        // When multiple policies would block, they collapse to a single Block
        // with the most restrictive reason.
        if matches!(verdict, PolicyVerdict::Block { .. }) {
            // At least one policy must be exceeded
            let has_exceeded = policies.iter().any(|p| {
                p.time_limit_seconds()
                    .map_or(false, |limit| elapsed >= limit as i64)
            });
            prop_assert!(has_exceeded);
        }
    }

    #[test]
    fn extended_usage_never_exceeds_original_plus_extra(
        app_id in app_id_strategy(),
        policy in any_time_limit_policy(),
        elapsed in 0i64..86400,
        now in datetime_strategy(),
    ) {
        // When a TimeLimit policy has extra_seconds, the effective limit
        // must be time_limit_seconds + extra_seconds, never more.
        let effective_limit = policy.time_limit_seconds.unwrap()
            + policy.extra_seconds;
        // If elapsed exceeds effective_limit, the verdict must allow
        // extension (or already be Extended).
        // ...
    }
}
```

### Strategies

```rust
fn app_id_strategy() -> impl Strategy<Value = AppId> {
    "[a-zA-Z][a-zA-Z0-9._-]{1,32}"  // match AppId validation
}

fn any_policy() -> impl Strategy<Value = Policy> {
    prop_oneof![
        any_block_policy(),
        any_time_limit_policy(),
        any_notify_policy(),
    ]
}
```

### Why property-based tests?

- **Covers edge cases** hand-written tests miss (empty policy list,
  boundary-second limits, overlapping category+app policies)
- **Documents invariants** as executable properties — readers know what the
  policy engine guarantees
- **Regression resistant** — add one invariant, catch thousands of potential
  failures

### Contract for property tests

- All property tests live in `policy/core/tests/` (integration-style, not
  `#[cfg(test)] mod` inside the module)
- The pure `evaluate()` function is tested only via property tests — no
  hand-written example-based tests for basic cases
- Hand-written tests cover actor integration (EnforcerActor + store), not pure
  evaluation logic

---

## Integration Tests for D-Bus Server

The D-Bus daemon interface (`org.wellbeing.v1.Daemon`) is the primary API
contract and MUST be tested with a real zbus connection in loopback mode.

### Pattern

```rust
use zbus::connection::Builder;
use zbus::blocking;

#[tokio::test]
async fn list_policies_returns_caller_policies() {
    // GIVEN: daemon with known policies in store
    let (pool, _) = test_db().await;
    let iface = DaemonInterface::new(pool.clone());
    let conn = Builder::loopback()
        .unwrap()
        .serve_at("/org/wellbeing/Daemon", iface)
        .unwrap()
        .build()
        .await
        .unwrap();

    // WHEN: connecting as uid 1000 and calling ListPolicies
    let proxy = DaemonProxy::new(&conn).await.unwrap();
    let policies = proxy.list_policies(0).await.unwrap();

    // THEN: returns only policies owned by uid 1000
    assert!(policies.iter().all(|p| p.owner_id == 1000));
}

/// Create a test database with seeded policies for uid 1000.
async fn test_db() -> DbPool {
    let builder = StoreBuilder::new_in_memory().await.unwrap();
    let pool = builder.build().await.unwrap();

    let mut conn = pool.get().await.unwrap();
    diesel::insert_into(policies::table)
        .values(&[
            (name: "test1", owner_id: 1000, created_by: 1000, kind: 0, schedule_json: "{}"),
            (name: "test2", owner_id: 1001, created_by: 1001, kind: 1, schedule_json: "{}"),
        ])
        .execute(&mut conn)
        .await
        .unwrap();

    pool
}
```

### What to test

| Test scenario                                     | Verifies               |
| ------------------------------------------------- | ---------------------- |
| `ListPolicies` returns only caller's policies     | RBAC filtering         |
| `ListPolicies` as root returns all policies       | Admin override         |
| `CreatePolicy` sets correct `created_by`          | Ownership tracking     |
| `DeletePolicy` rejects cross-user delete          | Permission enforcement |
| `GetDailyUsage` scopes to caller uid              | Data isolation         |
| `BlockStateChanged` signal emitted on block       | Signal contract        |
| `DailyUsageChanged` signal emitted on event write | Signal contract        |
| Method call from unauthenticated connection       | Error handling         |
| Concurrent policy CRUD from two users             | Isolation              |

---

## Crash Recovery Tests

The daemon must survive process restarts without data loss or duplicate
intervals. These tests simulate crash scenarios.

### Scenario: Daemon restart during active block

```rust
#[tokio::test]
async fn daemon_restart_preserves_block_state() {
    // GIVEN: daemon was running, app blocked, then daemon crashed
    let events = vec![
        PlatformEvent::WindowFocused {
            app_id: AppId::new("firefox").unwrap(),
            title: WindowTitle::new("Mozilla Firefox").unwrap(),
            pid: Pid::new(100),
            uid: Uid::new(1000),
            overlay_shown: true,  // plugin still rendering overlay
        },
    ];
    let (platform, stream) = MockPlatform::from_events(events);

    // WHEN: new daemon instance starts, reads CurrentSession,
    //      plugin reports overlay_shown=true
    let mut enforcer = EnforcerActor::new(platform, pool.clone(), clock.clone());
    enforcer.reconcile_on_startup().await;

    // THEN: daemon re-adopts the block state without duplicate Unfocused
    let state = enforcer.block_state().await;
    assert!(state.is_blocked("firefox"));
    // No duplicate WindowFocused or spurious Unfocused in events table
    let event_count: i64 = conn.scalar("SELECT COUNT(*) FROM events").await;
    assert_eq!(event_count, 1);  // only the original WindowFocused
}
```

### Scenario: Suspend during focus interval

```rust
#[tokio::test]
async fn suspend_closes_open_interval() {
    // GIVEN: app focused, no focus switch yet
    // WHEN: PrepareForSleep signal arrives
    // THEN: Slept is written, interval accumulated,
    //       resume does not accrue wall-clock time
}
```

### Scenario: SIGTERM during event write

```rust
#[tokio::test]
async fn sigterm_flushes_open_interval() {
    // GIVEN: app focused, event written but LoggedOut not yet sent
    // WHEN: SIGTERM delivered
    // THEN: LoggedOut is inserted before process exits
}
```

### Test infrastructure

- All crash recovery tests use `VirtualClock` and in-memory SQLite
- `MockPlatform` replays recorded event traces including the `overlay_shown`
  flag
- The `EnforcerActor` exposes a `reconcile_on_startup()` method that the
  production `main.rs` also calls on restart
- Tests verify final DB state by reading the events table directly

---

## Time Abstraction (VirtualClock)

Time-dependent logic — policy evaluation, grace timers, session tracking, data
retention pruning — cannot be tested deterministically with wall-clock time. A
`Clock` trait is injected into every actor and pure function that needs `now()`:

```rust
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use chrono::{DateTime, Utc, Duration};

/// Abstracts the current time. Production: SystemClock wraps Utc::now().
/// Test: VirtualClock allows advancing time deterministically.
pub trait Clock: Send + Sync + 'static {
    fn now(&self) -> DateTime<Utc>;
}

/// Production clock. Single zero-sized struct — zero runtime cost.
pub struct SystemClock;
impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> { Utc::now() }
}

/// Test clock. Call advance() to move time forward.
/// All clones share the same underlying Arc — advancing one advances all.
///
/// Uses AtomicI64 (epoch millis) instead of std::sync::Mutex to avoid
/// blocking a tokio runtime thread when now() is called from an actor.
/// Lock-free, zero-wait — safe in any async context.
pub struct VirtualClock {
    now: Arc<AtomicI64>,
}

impl VirtualClock {
    pub fn new(initial: DateTime<Utc>) -> Self {
        Self { now: Arc::new(AtomicI64::new(initial.timestamp_millis())) }
    }
    /// Advance the clock. Subsequent now() calls return the advanced time.
    pub fn advance(&self, duration: Duration) {
        self.now.fetch_add(duration.as_millis() as i64, Ordering::Relaxed);
    }
}

impl Clock for VirtualClock {
    fn now(&self) -> DateTime<Utc> {
        let millis = self.now.load(Ordering::Relaxed);
        DateTime::from_timestamp_millis(millis).expect("VirtualClock epoch overflow")
    }
}
```

### Policy Evaluation — Pure Function

Policy evaluation is a **pure domain function** — no clock, no DB pool, no
struct. It accepts `now` as an explicit parameter:

```rust
pub fn evaluate(
    app_id: &AppId,
    policies: &[Policy],       // pre-filtered by data layer
    elapsed_usage: Duration,    // total_seconds from daily_usage
    now: DateTime<Utc>,         // explicit — no Clock dependency
) -> PolicyVerdict
```

The `EnforcerActor` (which does have DB access) calls the data layer first to
load matching policies, then passes them to `evaluate()`.

### Usage in Tests

```rust
#[test]
fn given_category_policy_exceeded_when_app_blocked_then_block() {
    // GIVEN — policies pre-loaded as domain types
    let cat_policy = Policy::Category {
        id: PolicyId::new(1),
        name: "Social limit".into(),
        config: PolicyConfig {
            kind: PolicyKind::TimeLimit,
            time_limit_seconds: Some(1800),
            extra_seconds: 600,
            schedule: TimeWindow::always(),
        },
        category_id: CategoryId::new(3),
    };
    let app_policy = Policy::App {
        id: PolicyId::new(2),
        name: "Firefox limit".into(),
        config: PolicyConfig {
            kind: PolicyKind::TimeLimit,
            time_limit_seconds: Some(3600),
            extra_seconds: 600,
            schedule: TimeWindow::always(),
        },
        app_id: AppId::unchecked("firefox"),
    };
    let now = Utc.with_ymd_and_hms(2026, 7, 7, 23, 30, 0).unwrap();

    // WHEN — 2000s used: exceeds category limit (1800), within app limit (3600)
    let verdict = evaluate(
        &AppId::unchecked("firefox"),
        &[cat_policy, app_policy],
        Duration::seconds(2000),
        now,
    );

    // THEN — category policy triggers first
    assert!(matches!(verdict, PolicyVerdict::Block { reason: BlockReason::CategoryTimeLimit(_), .. }));
}
```

### Contract

- `VirtualClock` implements `Clone` — each clone shares the same
  `Arc<AtomicI64>`. Advancing one advances all. This allows passing the same
  logical clock to multiple actors in a test.
- All actors that issue time-stamped DB writes (`TrackerActor`, `EnforcerActor`,
  prune loop) **must** take a `Clock` parameter — they need deterministic time
  for testable timestamp assertions.
- Pure domain logic (`evaluate`, `TimeWindow::is_active`) should accept an
  explicit `now: DateTime<Utc>` parameter. Only actors with DB access accept a
  `Clock` (they call it once and pass the value down).
