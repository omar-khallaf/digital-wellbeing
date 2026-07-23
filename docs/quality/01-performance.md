# Performance & Safety Rules

## Memory

### Zero-Allocation Hot Path

Window events arrive as D-Bus signal payloads and are parsed directly from the
zbus message without copying. Zero allocations beyond the event payload.

Event parsing converts the D-Bus FocusChanged payload directly into a
PlatformEvent::WindowFocused by constructing the strongly-typed newtypes (AppId,
WindowTitle, Pid, Uid) from the payload fields. The only allocation is the
app_id string inside AppId::new. Unfocused events require no string allocation
at all.

Parsing avoids regex entirely. Compositor event parsing uses str::split and
str::starts_with rather than compiled patterns. Compiled regex on D-Bus args is
forbidden — it would allocate and compile per call.

### Preallocation

- Vecs with known batch sizes are pre-allocated using with_capacity (e.g., trait
  bounds, policy lists loaded once).
- The compositor socket read buffer reuses a BytesMut or Vec<u8> with capacity
  4096 instead of allocating per read.
- The app classification HashMap is pre-sized to the expected number of tracked
  apps (common: 50 to 200 unique app_ids).

### LRU Cache Strategy

| Cache                       | Key   | Max Size | TTL              |
| --------------------------- | ----- | -------- | ---------------- |
| AI classification           | AppId | 200      | 60 seconds       |
| App metadata (display name) | AppId | 500      | Session lifetime |
| Category membership         | AppId | 200      | 5 minutes        |

Use the lru crate or a simple HashMap plus VecDeque eviction. Do not pull in a
heavy caching library.

### No Clone in Hot Path

- tokio::sync::watch::Receiver::borrow() returns a Ref with zero allocation.
  Never clone the watched state in a render loop.
- UI reads the borrow, converts to render primitives, drops the borrow.
- Event payloads move (not clone) through channels.

## CPU

### Hot Path Budget

Single event parse plus state transition must complete in under 1 microsecond
(measured with std::hint::black_box benchmarks). The receive, process, notify
loop iteration (in-memory state machine update only) completes in under 10
microseconds.

Persist is not on the hot path. Each event is written to SQLite, but the DB
insert is dispatched as an async conn.execute await (WAL mode) — it is awaited
outside the measured parse/state cycle, never blocking it. A single WAL insert
is roughly 10 to 100 microseconds, which is fine: it is decoupled from event
processing and coalesced by the 16ms render/notify loop. Do not include the DB
write in the sub-10-microsecond budget.

### Classification Cache

Classifying a window involves an app_categories DB lookup (PK point query), AI
classification as fallback (cached 60 seconds), and pattern matching on title
(for browser tabs). The AI result per AppId is cached for 60 seconds.

### Event-Driven Writes (Buffered Flush)

Events are not written individually. Focus switches are buffered in memory
inside `EnforcerActor.event_buffer` (a bounded 10k FIFO `EventBuffer` in
`crates/daemon/src/blocking/buffer.rs`). A minute-ticker aligned to wall-clock
minute boundaries triggers periodic flushes.

Flush triggers:

- **Count threshold**: buffer contains 100 or more events.
- **Timer boundary**: the minute-ticker fires at the next wall-clock minute
  boundary, regardless of buffer occupancy.

When a flush fires, all buffered events are written in a single batch INSERT
inside a `conn.transaction()`. After a successful commit, a
`DailyUsageChanged` D-Bus signal is emitted to notify the GUI. An empty flush
(no events in the buffer) is a no-op — zero DB writes and no signal emission.

Shutdown and suspend paths force an immediate flush so no events are lost.
When the daemon resumes from suspend, any accumulated events in the buffer are
flushed on the next tick or count trigger.

### inline Policy

Mark hot domain functions with inline:

- Classification matches (AppPattern::matches(AppId))
- Policy evaluation (evaluate — pure domain function)
- Newtype accessors (AppId::as_str(), DurationSecs::get())
- Time window checks (TimeWindow::is_active())

Let the compiler decide on the rest.

### cfg(debug_assertions) Guards

Expensive invariant checks only in debug builds:

- Session start time is <= current system time
- Daily totals are all under 86400 \* 365 seconds

Zero cost in release builds.

### No Regex in Hot Path

Compile regex patterns at module init with once_cell::sync::Lazy or
std::sync::OnceLock. For simple patterns (compositor event parsing), use
str::split and str::starts_with — they are 10 to 50 times faster than regex.

## Async Discipline

### No std::sync::Mutex in Async Context

tokio::sync::Mutex yields instead of blocking. std::sync::Mutex blocks the
entire thread (including other tasks on the same runtime). Forbidden.

### Watch Channel, Don't Push

UI polls state via borrow(), zero-copy. Clone the entire state every frame.

### Decouple Event Rate from Render Rate

UI render loop uses a blocking receive with timeout, not a busy-poll. The loop
blocks until an event arrives or a 16ms heartbeat fires. Under zero events the
loop sleeps on recv() — zero CPU wakeups. When an event arrives, it drains any
accumulated events into a batch, applies them to state, and renders a single
frame. Never 1:1 event-to-render. A burst of 50 events produces exactly 1
render. The 16ms timeout acts as a heartbeat guard.

### Bounded Channels for Commands

mpsc channel with capacity 32 for all actor command channels. Backpressure on
full channel is a signal that the actor is overloaded — log and drop.

Window events use mpsc channel with capacity 256 with send().await — bounded
backpressure. If the consumer is slow, the producer yields until the channel
drains, limiting transient latency to under 16ms (one render frame). The 16ms
decoupled render loop coalesces bursts, so the consumer always catches up within
one frame. This prevents unbounded memory growth from compositor event floods
(rapid alt-tabbing, window storms) while guaranteeing no event loss — dropping a
WindowFocused/Unfocused pair would corrupt an interval.

### Profiling Requirements

Before any performance-related PR, provide before/after measurements:

- perf stat for CPU cache misses and branch mispredictions
- heaptrack or dhat-rs for allocation counts
- flamegraph for hot spots

Include the profiling command and results in the PR description.

## unsafe Policy

unsafe is explicitly forbidden unless:

1. Justified with a // SAFETY: comment containing a formal proof of safety.
2. Reviewed by at least one other contributor.
3. Wrapped in a safe function with no unsafe exposure in the public API.

Exceptions (with review):

- FFI to wayland-client protocol libraries
- SIMD-accelerated event parsing (benchmarked 2x+ improvement)
