# Testing Philosophy

## Tenet: Test Behavior, Not Structure

The type system enforces invariants (no raw string, no invalid states). Tests
exist to verify business rules and domain logic, not structural trivia.

### Bad — testing the compiler's job

A test that asserts AppId rejects empty input is redundant. The NonEmptyString
newtype already guarantees this at compile time for any code path. Testing it is
wasteful because any test that compiles must already pass — there is no path
where AppId accepts empty input.

### Good — testing the business rule

A test should construct a realistic scenario, trigger the behavior under test,
and assert on the outcome. For example, constructing a PolicyConfig::TimeLimit
with time_limit_minutes, extra_minutes, and a schedule, then calling evaluate()
with the policy and elapsed usage, and asserting on the returned PolicyVerdict
variant (e.g., PolicyVerdict::Block when elapsed exceeds limit).

## Pattern: Given-When-Then

Every test follows this structure explicitly:

GIVEN [preconditions / system state] WHEN [action / command / event] THEN
[expected outcome / domain events / state change]

## Assert Both Domain Events and Database State

Domain events verify that business logic reached the right decision. Database
state verifies that the decision was persisted correctly. Both are required —
events are ephemeral (they exit the actor boundary), while the DB is the
canonical record for reports and crash recovery.

A test might construct a TrackerState, feed it a WindowFocused event for an
Alacritty window with a zsh title, pid 1234, uid 1000, and overlay_shown false,
and then assert that the resulting domain events include WindowFocusChanged. It
then queries the database and asserts that exactly one event row exists for
Alacritty with event type WindowFocused.

Assert both the decision path (events) and the persistence path (DB rows); a
test that only checks events can pass while the DB write silently fails (e.g., a
connection error that is logged but swallowed).

## Sociable Tests, Not Isolated Tests

Do NOT mock value objects, entities, or same-module collaborators. Let them
interact naturally. Only mock at system boundaries:

| Mock-worthy                              | Not worth mocking             |
| ---------------------------------------- | ----------------------------- |
| Platform (via MockPlatform)              | AppId, Duration, Policy       |
| SQLite connection (in-memory via diesel) | PolicyVerdict, WindowTitle    |
| System clock (for time-dependent tests)  | SessionSummary, TrackingState |
| Compositor socket (internal Linux tests) | Classification rules          |

### MockPlatform Pattern

The primary test boundary is Platform. A concrete MockPlatform replays recorded
event traces and fakes all platform operations. It shares an event deque between
the returned platform handle and the stream, so the actor consumes exactly the
seeded events. show_overlay and hide_overlay are inlined directly — always
succeed, and record the last config for assertions. All feature-level
integration tests use MockPlatform. The Linux platform is tested via a separate
integration suite (for compositor-specific behavior, a MockCompositor exists
internal to platform/linux/).

## Test Organization

Each feature directory has its own tests:

Unit tests (cfg(test) mod tests inside each module): Test domain logic, state
machines, pure functions. Sociable within the module. Integration tests
(tests/): Test feature boundaries, async actor behavior with MockPlatform, full
pipeline from event to store.

## What NOT to Test

- Structural invariants guaranteed by the type system (AppId rejects empty)
- Simple field accessors / getters
- Third-party behavior (SQLite, gpui, tokio channels)
- Code that is "too simple to break" — classification overrides that are a
  HashMap::get, for example

Use judgment: if the test would test the compiler or the standard library,
delete it.

## Property-Based Testing for the Policy Engine

The evaluate() function is a pure function mapping (app_id, policies,
elapsed_usage, now) to PolicyVerdict — an ideal candidate for property-based
testing with proptest or bolero.

### Invariants to verify

Proptest generates random app_id strings matching the AppId validation pattern,
random policy vectors of size 0 to 5, elapsed values from 0 to 86400 seconds
(0–24h), and random datetimes. It asserts that if no policy has a limit, the
verdict must not be Block. It asserts that when multiple policies would block,
they collapse to a single Block with the most restrictive reason. It asserts
that when a TimeLimit policy has extra_minutes, the effective limit must be
time_limit_minutes plus extra_minutes, never more. If elapsed exceeds
effective_limit, the verdict must allow extension (or already be Extended).

### Strategies

The app_id strategy generates strings matching the pattern for valid AppId
values — starting with a letter, followed by alphanumeric, dot, underscore, or
hyphen characters, 1 to 32 characters long. The any_policy strategy combines any
block policy, any time limit policy, and any notify policy.

### Why property-based tests?

- Covers edge cases hand-written tests miss (empty policy list, boundary-second
  limits, overlapping category+app policies)
- Documents invariants as executable properties — readers know what the policy
  engine guarantees
- Regression resistant — add one invariant, catch thousands of potential
  failures

### Contract for property tests

- All property tests live in policy/core/tests/ (integration-style, not
  cfg(test) mod inside the module)
- The pure evaluate() function is tested only via property tests — no
  hand-written example-based tests for basic cases
- Hand-written tests cover actor integration (EnforcerActor + store), not pure
  evaluation logic

## Integration Tests for D-Bus Server

The D-Bus daemon interface (org.wellbeing.v1.Controller) is the primary API
contract and MUST be tested with a real zbus connection in loopback mode.

### Pattern

A D-Bus integration test creates an in-memory test database with seeded policies
for uid 1000, constructs the daemon interface with the pool, builds a loopback
zbus connection serving at the daemon object path, connects as uid 1000 via a
proxy, calls list_policies(0), and asserts that the returned policies are all
owned by uid 1000.

### What to test

| Test scenario                                   | Verifies               |
| ----------------------------------------------- | ---------------------- |
| ListPolicies returns only caller's policies     | RBAC filtering         |
| ListPolicies as root returns all policies       | Admin override         |
| CreatePolicy sets correct created_by            | Ownership tracking     |
| DeletePolicy rejects cross-user delete          | Permission enforcement |
| GetDailyUsage scopes to caller uid              | Data isolation         |
| BlockStateChanged signal emitted on block       | Signal contract        |
| DailyUsageChanged signal emitted on event write | Signal contract        |
| Method call from unauthenticated connection     | Error handling         |
| Concurrent policy CRUD from two users           | Isolation              |

## Crash Recovery Tests

The daemon must survive process restarts without data loss or duplicate
intervals. These tests simulate crash scenarios.

### Scenario: Daemon restart during active block

A test seeds an event trace with a WindowFocused event for firefox with
overlay_shown true and pid 100, uid 1000. It builds a MockPlatform from those
events, creates a new EnforcerActor with a cloned pool and clock, calls
reconcile_on_startup(), and asserts that the block state for firefox is still
blocked. It also asserts that the events table contains exactly one row — the
original WindowFocused — with no duplicates.

### Scenario: Suspend during focus interval

A test verifies that when a PrepareForSleep signal arrives while an app is
focused and no focus switch has yet occurred, a Slept event is written, the
interval is accumulated, and resume does not accrue wall-clock time.

### Scenario: SIGTERM during event write

A test verifies that when SIGTERM is delivered while an event has been written
but LoggedOut has not yet been sent, a LoggedOut event is inserted before the
process exits.

### Test infrastructure

- All crash recovery tests use VirtualClock and in-memory SQLite
- MockPlatform replays recorded event traces including the overlay_shown flag
- The EnforcerActor exposes a reconcile_on_startup() method that the production
  main.rs also calls on restart
- Tests verify final DB state by reading the events table directly

## Time Abstraction (VirtualClock)

Time-dependent logic — policy evaluation, grace timers, session tracking, data
retention pruning — cannot be tested deterministically with wall-clock time. A
Clock trait is injected into every actor and pure function that needs now().

Production uses SystemClock which wraps Utc::now(). Test uses VirtualClock which
allows advancing time deterministically. VirtualClock exposes an advance method
that moves time forward by a given duration. It uses AtomicI64 (epoch millis)
instead of a mutex to avoid blocking a tokio runtime thread when now() is called
from an actor — lock-free, zero-wait, safe in any async context.

### Policy Evaluation — Pure Function

Policy evaluation is a pure domain function — no clock, no DB pool, no struct.
It accepts now as an explicit parameter:

evaluate(app_id, policies, elapsed_usage, now) -> PolicyVerdict

The EnforcerActor (which does have DB access) calls the data layer first to load
matching policies, then passes them to evaluate().

### Usage in Tests

Construct a PolicyConfig::TimeLimit for a category (with category_id) and one
for an app (with app_id). Pass both to evaluate() with elapsed usage below the
app limit but above the category limit — the category policy triggers first.

### Contract

- VirtualClock implements Clone — each clone shares the same Arc<AtomicI64>.
  Advancing one advances all. This allows passing the same logical clock to
  multiple actors in a test.
- All actors that issue time-stamped DB writes (EnforcerActor, prune loop)
  prune loop) must take a Clock parameter — they need deterministic time for
  testable timestamp assertions.
- Pure domain logic (evaluate, TimeWindow::is_active) should accept an explicit
  now: DateTime<Utc> parameter. Only actors with DB access accept a Clock (they
  call it once and pass the value down).
