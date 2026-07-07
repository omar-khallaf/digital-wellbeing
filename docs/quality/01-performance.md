# Performance & Safety Rules

## Memory

### Zero-Allocation Hot Path

Window events arrive as D-Bus signal payloads and are parsed directly from the
zbus message without copying. **Zero allocations beyond the event payload.**

```rust
// GOOD: Parse D-Bus FocusChanged signal payload directly, no copy
fn parse_focus_changed(window: Option<WindowInfo>) -> Option<PlatformEvent> {
    match window {
        Some(info) => {
            Some(PlatformEvent::WindowFocused {
                app_id: AppId::new(info.app_id),   // one alloc: app_id string
                title: WindowTitle::new(info.title),
                pid: Pid::new(info.pid),
                uid: Uid::new(info.uid),
                overlay_shown: info.overlay_shown,
            })
        }
        None => Some(PlatformEvent::Unfocused),
    }
}

// BAD: Regex on D-Bus args
fn parse_event_bad(raw: &str) -> Result<CompositorEvent, ParseError> {
    let re = Regex::new(r"^WindowFocused\{.*").unwrap(); // compiled per call!
    // ...
}
```

### Preallocation

- `Vec::with_capacity(n)` where batch size is known (e.g., trait bounds, policy
  lists that are loaded once).
- Buffer for compositor socket reads: reuse a `BytesMut` or `Vec<u8>` with
  capacity 4096 instead of allocating per read.
- `HashMap` for app classification cache: pre-size to expected number of tracked
  apps (common: 50–200 unique app_ids).

### LRU Cache Strategy

| Cache                       | Key     | Max Size | TTL              |
| --------------------------- | ------- | -------- | ---------------- |
| AI classification           | `AppId` | 200      | 60s              |
| App metadata (display name) | `AppId` | 500      | Session lifetime |
| Category membership         | `AppId` | 200      | 5 min            |

Use `lru` crate or a simple `HashMap` + VecDeque eviction. Do not pull in a
heavy caching library.

### No Clone in Hot Path

- `tokio::sync::watch::Receiver::borrow()` returns a `Ref<T>` with **zero
  allocation**. Never `.clone()` the watched state in a render loop.
- UI reads the borrow, converts to render primitives, drops the borrow.
- Event payloads move (not clone) through channels.

## CPU

### Hot Path Budget

Single event parse + state transition must complete in **< 1 µs** (measured with
`std::hint::black_box` benchmarks). The receive → process → notify loop
iteration (in-memory state machine update only) in **< 10 µs**.

**Persist is not on the hot path.** Each event is written to SQLite, but the DB
insert is dispatched as an async `conn.execute().await` (WAL mode) — it is
awaited _outside_ the measured parse/state cycle, never blocking it. A single
WAL insert is ~10–100 µs, which is fine: it is decoupled from event processing
and coalesced by the 16ms render/notify loop. Do **not** include the DB write in
the < 10 µs budget.

### Classification Cache

Classifying a window involves:

1. `app_categories` DB lookup (PK point query)
2. AI classification (fallback, cached 60s)
3. Pattern matching on title (for browser tabs)

Cache the AI result per `AppId` for 60 seconds:

```rust
struct ClassificationCache {
    app_id: AppId,
    category: WindowCategory,
    cached_at: Instant,
}
```

### Event-Driven Writes

Events are written on demand per user action (focus switch, block, extra time
grant). Write frequency is bounded by user interaction rate (max ~1 per 100ms).
SQLite in WAL mode handles individual inserts at this rate without batching. No
periodic flush timer, no accumulation buffer, no batch transaction.

### `#[inline]` Policy

Mark hot domain functions with `#[inline]`:

- Classification matches (`AppPattern::matches(AppId)`)
- Policy evaluation (`evaluate` — pure domain function)
- Newtype accessors (`AppId::as_str()`, `DurationSecs::get()`)
- Time window checks (`TimeWindow::is_active()`)

Let the compiler decide on the rest.

### `#[cfg(debug_assertions)]` Guards

Expensive invariant checks only in debug builds:

```rust
#[cfg(debug_assertions)]
fn assert_valid_state(&self) {
    assert!(self.session_started <= SystemTime::now());
    assert!(self.daily_total.values().all(|d| d.as_secs() < 86400 * 365));
}
```

Zero cost in release builds.

### No Regex in Hot Path

Compile regex patterns at module init with `once_cell::sync::Lazy` or
`std::sync::OnceLock`:

```rust
static BROWSER_TAB_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^(.*) — (.* Firefox|Google Chrome|Chromium)$").unwrap()
});
```

For simple patterns (compositor event parsing), use `str::split*` and
`str::starts_with` — they are 10–50x faster than regex.

## Async Discipline

### no `std::sync::Mutex` in Async Context

`tokio::sync::Mutex` yields instead of blocking. `std::sync::Mutex` blocks the
entire thread (including other tasks on the same runtime). **Forbidden.**

### Watch Channel, Don't Push

```rust
// GOOD: UI polls state via borrow(), zero-copy
let state = state_rx.borrow();
ui.render(state);

// BAD: cloning the entire state every frame
let state = state_rx.borrow().clone();
```

### Decouple Event Rate from Render Rate

UI render loop — use a **blocking receive with timeout**, not a busy-poll:

```rust
loop {
    // Block until event arrives OR 16ms heartbeat fires.
    // Under zero events the loop sleeps on recv() — zero CPU wakeups.
    let event = tokio::time::timeout(
        Duration::from_millis(16),
        event_rx.recv()
    ).await;

    // Drain any accumulated events (batch render)
    let batch: Vec<PlatformEvent> = {
        let mut batch = Vec::with_capacity(8);
        if let Ok(Some(event)) = event {
            batch.push(event);
            // Collect events that arrived during the render of the previous frame
            while let Some(ev) = event_rx.try_recv().ok() {
                batch.push(ev);
            }
        }
        batch
    };

    // Apply events to state
    for event in &batch {
        self.state.apply(event);
    }

    // Single render per frame regardless of event count
    self.render_frame(&self.state);
}
```

Never 1:1 event→render. A burst of 50 events should produce exactly 1 render.
The 16ms timeout acts as a heartbeat guard — if no events arrive, the loop still
wakes up periodically to handle any scheduled housekeeping, but the `recv()`
call blocks the runtime task efficiently (zero polling, zero yield overhead).

### Bounded Channels for Commands

`mpsc::channel(32)` for all actor command channels. Backpressure on full channel
is a signal that the actor is overloaded — log and drop.

Window events use `mpsc::channel(256)` with `send().await` — bounded
backpressure. If the consumer is slow, the producer yields until the channel
drains, limiting transient latency to <16ms (one render frame). The 16ms
decoupled render loop coalesces bursts, so the consumer always catches up within
one frame. This prevents unbounded memory growth from compositor event floods
(rapid alt-tabbing, window storms) while guaranteeing no event loss — dropping a
`WindowFocused`/`Unfocused` pair would corrupt an interval.

### Profiling Requirements

Before any performance-related PR, provide before/after measurements:

- `perf stat` for CPU cache misses and branch mispredictions
- `heaptrack` or `dhat-rs` for allocation counts
- `flamegraph` for hot spots

Include the profiling command and results in the PR description.

## `unsafe` Policy

`unsafe` is **explicitly forbidden** unless:

1. Justified with a `// SAFETY:` comment containing a formal proof of safety.
2. Reviewed by at least one other contributor.
3. Wrapped in a safe function with no `unsafe` exposure in the public API.

Exceptions (with review):

- FFI to wayland-client protocol libraries
- SIMD-accelerated event parsing (benchmarked 2x+ improvement)
