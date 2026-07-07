# Blocking Enforcement Design

## Core Principle: User Choice, Not Automatic Action

The system never automatically closes or terminates applications. Instead:

1. When a policy triggers, the **EnforcerActor** evaluates **before** any event
   is persisted. If the app is blocked, only the **overlay** is shown — no
   `WindowFocused` event is written, so the blocked app never enters the event
   log.
2. If the previous app has an open focus interval, a **Unfocused** event closes
   it (this is interval management, not block enforcement).
3. The overlay presents options to the user.
4. The user's choice determines the next action.

Enforcement is **overlay-only**. The blocked app continues running but the
overlay traps all input, making it impossible to interact with the window. This
keeps the compositor path simple (no process signal handling) and eliminates the
need for capability probing (CAP_SYS_PTRACE) and crash recovery of process
state.

---

## TimeLimitedApp — Domain Model

The `EnforcerActor` constructs this per-policy after receiving a `Block` verdict
to determine overlay options for the blocking policy:

```rust
/// Domain enum representing the time-limited state of an app against
/// a single policy. Constructed from daily_usage + policy config.
pub enum TimeLimitedApp {
    /// Normal regime: within policy time_limit.
    /// limit is policy.time_limit_seconds.
    Normal(used: i64, limit: i64),
    /// Extended regime: user already granted extra time.
    /// limit is policy.time_limit_seconds + policy.extra_seconds.
    Extended(used: i64, limit: i64),
}

impl TimeLimitedApp {
    /// Remaining time before limit is reached.
    pub fn remaining(&self) -> i64 {
        match self {
            Normal(used, limit) | Extended(used, limit) => limit - used,
        }
    }

    /// Whether the user can extend time further (only once, in Normal regime).
    pub fn can_extend(&self) -> bool {
        matches!(self, Normal(..))
    }

    /// Max limit for this regime.
    pub fn effective_limit(&self) -> i64 {
        match self {
            Normal(_, limit) | Extended(_, limit) => *limit,
        }
    }
}
```

**Note:** `PolicyKind::Block` (direct block, no time tracking) has no
`time_limit_seconds` — it blocks unconditionally when active. For that kind the
overlay shows only [Close] (no [Extra] button), and no tracked state is
constructed.

### TrackedApp — Unified Domain Model

The new domain model unifies both blocking and notify-only tracking:

```rust
/// Unifies tracked state for both blocking and notify-only policies.
pub enum TrackedApp {
    /// Hard deadline with optional extension (TimeLimit policies).
    TimeLimited(TimeLimitedApp),
    /// Tracked usage with notification reminders (Notify policies).
    TimeTracked(TimeTrackedApp),
}

impl TrackedApp {
    pub fn used(&self) -> Duration { /* delegates to inner */ }
    pub fn remaining(&self) -> Duration { /* delegates to inner */ }
}
```

Resolving from DB row + policy config:

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

### TimeTrackedApp — Notify-Only Tracked State

```rust
/// Notify policy state — simple struct, no state machine.
/// Notification scheduling is ephemeral (EnforcerActor timers), not persisted.
pub struct TimeTrackedApp {
    pub used: Duration,
    pub limit: Duration,
}

impl TimeTrackedApp {
    pub fn remaining(&self) -> Duration {
        (self.limit - self.used).max(Duration::ZERO)
    }

    pub fn is_exceeded(&self) -> bool {
        self.used >= self.limit
    }
}
```

### Overlay Action Availability by State

| TrackedApp              | Overlay buttons     | Behaviour                                      |
| ----------------------- | ------------------- | ---------------------------------------------- |
| `TimeLimited(Normal)`   | [Extra (N)] [Close] | N = extra seconds from policy                  |
| `TimeLimited(Extended)` | [Close] only        | Already extended, no further extension allowed |
| `TimeTracked(...)`      | N/A                 | Notify policies never show overlays            |

---

## Blocking Flow

### Policy Evaluation — Pure Domain Function

```rust
/// Evaluates ALL policies relevant to an app. AND semantics:
/// - Block wins over everything — if ANY policy blocks, the app is blocked.
/// - Notify verdicts stack as advisory only (first Notify determines payload).
/// - The first blocking policy determines the overlay reason.
pub fn evaluate(
    app_id: &AppId,
    policies: &[Policy],          // pre-filtered by DB (app_id + categories)
    elapsed_usage: Duration,       // total_seconds from daily_usage
    now: DateTime<Utc>,
) -> PolicyVerdict
```

**Filtering (data layer, before `evaluate()` is called):**

```sql
SELECT * FROM policies
WHERE active = 1
  AND (app_id = ? OR category_id IN (?, ?, ...))
```

The `EnforcerActor` resolves the app's categories first (via `app_categories`
table), then queries only matching policies. The domain function never loads all
policies.

**AND semantics:** The function iterates all matching policies:

- `PolicyKind::Block` → immediate `PolicyVerdict::Block` (unconditional, no time
  tracking)
- `PolicyKind::TimeLimit`, `remaining ≤ 0` → `PolicyVerdict::Block`
- `PolicyKind::Notify`, `remaining ≤ 0` → first Notify triggers
  `PolicyVerdict::Notify`; subsequent Notify violations are collected but don't
  override the first (Block still wins)
- All pass → `PolicyVerdict::Ok`
- Notify triggered but no Block → `PolicyVerdict::Notify`

### Full Flow

```
WindowFocused { B } arrives from plugin (PlatformEvent)
        │
        ▼
EnforcerActor (gate — evaluates BEFORE any DB write):
  1. Resolve B's app_id → Vec<CategoryId>     ← app_categories table
  2. Query policies WHERE active
     AND (app_id = ? OR category_id IN (...))
  3. Query B's daily_usage (total_seconds, extended)
        │
        ▼
  4. evaluate(B, &policies, elapsed_usage, now)     ← PURE DOMAIN FN
        │
        ├─ PolicyVerdict::Block
        │       │
        │       ▼
        │   a. Check in-memory focus state — if previous app A has open interval:
        │      INSERT Unfocused                          ← closes A's interval
        │      (TrackerActor accumulate_interval() closes A via in-memory focus state)
        │   b. Build ShowOverlayConfig {
        │        reason, policy_id,
        │        available_actions: match policy.kind {
        │          Block     → [Close]
        │          TimeLimit → app_state(usage, config).can_extend()
        │                       ? [Extra, Close] : [Close]
        │        }
        │      }
        │   c. platform.show_overlay(config)           ← fire-and-forget D-Bus
        │   d. Do NOT write WindowFocused for B
        │      (B never enters event log — no interval to close)
        │
        ├─ PolicyVerdict::Notify
        │       │
        │       ▼
        │   a. INSERT Unfocused                          ← closes previous A's interval
        │   b. INSERT WindowFocused for B              ← opens B's interval
        │      (trigger accumulates A, opens B)
        │   c. platform.notify("Limit reached", ...)   ← D-Bus notification
        │   d. Start notification repeat timer:
        │        if repeat_interval > 0:
        │          delay = repeat_interval - ((used - limit) % repeat_interval)
        │          spawn tokio sleep(delay)
        │          When timer fires → if B still focused, notify again
        │   e. Start limit timer for other policies
        │
        └─ PolicyVerdict::Ok
                │
                ▼
            a. INSERT Unfocused                          ← closes previous A's interval
            b. INSERT WindowFocused for B              ← opens B's interval
               (trigger accumulates A, opens B)
            c. Calculate remaining time:
                 if Normal(used, limit):  rem = limit - used
                 if Extended(used, limit+extra): rem = (limit+extra) - used
               Spawn tokio sleep(rem). When it fires:
                 re-evaluate B; if limit exceeded → show overlay
```

**Key properties:**

- Policy evaluation happens **before** any event reaches the DB. If blocked, no
  `WindowFocused` is written at all.
- The `Unfocused` written during a block closes the **previous** app's interval
  (A), not the blocked app's (B never had one).
- **Timer-based re-triggering:** After a non-blocked app gains focus, a tokio
  sleep task fires when the policy limit would be reached. This catches limit
  expiry during continuous single-app use, not just on focus switches.
- **Notify is non-blocking:** The app's focus interval proceeds normally.
  Notifications are advisory only — delivered via `platform.notify()` which
  calls `org.freedesktop.Notifications` over D-Bus.
- If the daemon crashes between writing `Unfocused` (step a) and showing the
  overlay (step c), no tracked time is lost — the previous interval is already
  closed. On restart, the next focus event re-evaluates naturally.

---

## Limit Timer

When an app passes policy check and focus is granted (WindowFocused persisted),
the EnforcerActor spawns a tokio sleep task that fires when the policy limit
would be reached. This catches limit expiry during **continuous single-app
use**, not just on focus switches.

### Timer Calculation

```rust
/// Calculate remaining seconds until the policy limit is reached.
/// Returns 0 if the limit is already exceeded (should not happen
/// since evaluate() already checked).
fn remaining_seconds(usage: &DailyUsageRow, policy: &PolicyConfig) -> u64 {
    let limit = if usage.extended {
        policy.time_limit_seconds.unwrap_or(0) + policy.extra_seconds
    } else {
        policy.time_limit_seconds.unwrap_or(0)
    };
    (limit - usage.total_seconds).max(0) as u64
}
```

### Timer Lifecycle

```
App gains focus (WindowFocused persisted)
        │
        ▼
EnforcerActor:
  1. Calculate remaining = remaining_seconds(usage, policy)
  2. Start timer: tokio::spawn(sleep(remaining))
  3. Store JoinHandle in HashMap<AppId, JoinHandle<()>>
        │
        ├── Timer fires:
        │       │
        │       ▼
        │   EnforcerActor.on_limit_reached(app_id):
        │     1. Check if app is still focused (compare with active_window)
        │     2. Query current daily_usage
        │     3. Re-evaluate policy
        │     4. If Block → enforce_block()
        │     5. If Ok (policy changed) → start new timer
        │
        ├── User switches to different app:
        │       │
        │       ▼
        │   EnforcerActor cancels previous app's timer
        │   (JoinHandle::abort()), removes from HashMap
        │   New app gets its own timer
        │
        └── User extends time (grant_extension):
                │
                ▼
            Cancel old timer, start new timer
            with remaining = (limit + extra) - total_seconds
```

### Implementation Sketch — EnforcerActor

```rust
struct EnforcerActor {
    /// Active limit timers per app (TimeLimit policies).
    /// Cancelled on focus switch or extension.
    limit_timers: HashMap<AppId, tokio::task::JoinHandle<()>>,
    /// Active notification repeat timers per app (Notify policies).
    /// Cancelled on focus switch.
    notify_timers: HashMap<AppId, NotifyTimerState>,
    // ...
}

struct NotifyTimerState {
    policy_id: PolicyId,
    repeat_interval: Duration,
    last_notified_usage: Duration,
    handle: tokio::task::JoinHandle<()>,
}

impl EnforcerActor {
    // ── Limit Timer (TimeLimit policies) ──

    fn start_limit_timer(&mut self, app_id: AppId, remaining_secs: u64) {
        self.cancel_limit_timer(&app_id);
        let enforcer = self.weak_ref();
        let handle = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(remaining_secs)).await;
            if let Some(enforcer) = enforcer.upgrade() {
                enforcer.on_limit_reached(app_id).await;
            }
        });
        self.limit_timers.insert(app_id, handle);
    }

    fn cancel_limit_timer(&mut self, app_id: &AppId) {
        if let Some(handle) = self.limit_timers.remove(app_id) {
            handle.abort();
        }
    }

    async fn on_limit_reached(&mut self, app_id: AppId) {
        if self.active_window.as_ref().map(|w| w.app_id()) != Some(&app_id) {
            return;  // stale timer
        }
        let usage = self.fetch_daily_usage(&app_id).await;
        let policies = self.fetch_matching_policies(&app_id).await;
        let verdict = evaluate(&app_id, &policies, usage, Utc::now());
        if let PolicyVerdict::Block { .. } = verdict {
            self.enforce_block(&app_id, verdict, ...).await;
        }
    }

    // ── Notification Timer (Notify policies) ──

    fn start_notify_timer(&mut self, app_id: AppId, state: NotifyTimerState) {
        self.cancel_notify_timer(&app_id);
        let enforcer = self.weak_ref();
        let interval = state.repeat_interval;
        let handle = tokio::spawn(async move {
            tokio::time::sleep(interval).await;
            if let Some(enforcer) = enforcer.upgrade() {
                enforcer.on_notify_tick(app_id).await;
            }
        });
        self.notify_timers.insert(app_id, NotifyTimerState { handle, ..state });
    }

    fn cancel_notify_timer(&mut self, app_id: &AppId) {
        if let Some(timer) = self.notify_timers.remove(app_id) {
            timer.handle.abort();
        }
    }

    async fn on_notify_tick(&mut self, app_id: AppId) {
        if self.active_window.as_ref().map(|w| w.app_id()) != Some(&app_id) {
            return;
        }
        self.platform.notify("Limit reached", "You're still past the limit").await.ok();
        if let Some(mut timer) = self.notify_timers.get_mut(&app_id) {
            timer.last_notified_usage += timer.repeat_interval;
            let new_handle = self.spawn_notify_handle(app_id.clone(), timer.repeat_interval);
            timer.handle = new_handle;
        }
    }
}
```

The weak reference pattern (`weak_ref`) avoids holding a strong reference cycle
within the actor. The `EnforcerActor` uses `Arc<Mutex<...>>` interior mutability
(or an `mpsc` channel back to itself) to safely access actor state from the
spawned timer task.

---

## Notification Timer (Notify Policies)

When a Notify policy triggers and `notification_repeat_interval_seconds` is set,
the EnforcerActor starts a real-time timer that fires at the repeat interval
while the app remains focused. This catches the case where the user keeps using
the app past the limit — they get periodic reminders.

### Timer Calculation

The timer delay aligns to the next notification boundary based on the usage
known at the last focus event:

```
delay = repeat_interval - ((total_seconds - limit) % repeat_interval)
```

Example: limit=1h (3600s), repeat=5min (300s), usage at focus=3720s (1h2min) →
delay = 300 - ((3720 - 3600) % 300) = 300 - (120 % 300) = 300 - 120 = 180s

The timer fires after 180 real seconds. If the app is still focused at that
point, the usage has accumulated to ≥ 3900s (1h5min) and a new notification is
sent.

### Timer Lifecycle

```
App gains focus (WindowFocused persisted), evaluate returned Notify
        │
        ▼
EnforcerActor:
  1. platform.notify("Limit reached", ...)    ← immediate notification
  2. Store last_notified_usage = total_seconds
  3. If repeat_interval > 0:
       delay = repeat_interval - ((total_seconds - limit) % repeat_interval)
       if delay <= 0: delay = repeat_interval  // past multiple intervals
       Start timer: tokio::spawn(sleep(delay))
       Store JoinHandle in notify_timers map
        │
        ├── Timer fires:
        │       │
        │       ▼
        │   EnforcerActor.on_notify_tick(app_id):
        │     1. Check if app is still focused
        │     2. If yes: platform.notify(...)  ← re-notify
        │        last_notified_usage += repeat_interval
        │        Start new timer: tokio::spawn(sleep(repeat_interval))
        │     3. If no: stale timer, discard
        │
        ├── User switches to different app:
        │       │
        │       ▼
        │   Cancel notify_timer for app_id
        │   Cancel limit_timer for app_id
        │   New app re-evaluated on focus
        │
        └── User grants extension (TimeLimit only):
                // Notification timers are Notify-policy only;
                // Extension only applies to TimeLimit policies.
```

### Implementation — EnforcerActor with Notify Timers

```rust
struct EnforcerActor {
    /// Active limit timers per app (TimeLimit policies).
    limit_timers: HashMap<AppId, tokio::task::JoinHandle<()>>,
    /// Active notification repeat timers per app (Notify policies).
    notify_timers: HashMap<AppId, NotifyTimerState>,
    // ...
}

struct NotifyTimerState {
    policy_id: PolicyId,
    repeat_interval: Duration,
    last_notified_usage: Duration,  // total_seconds at last notification
    handle: tokio::task::JoinHandle<()>,
}

impl EnforcerActor {
    fn start_notify_timer(&mut self, app_id: AppId, state: NotifyTimerState) {
        // Cancel any existing notify timer for this app
        self.cancel_notify_timer(&app_id);

        let enforcer = self.weak_ref();
        let interval = state.repeat_interval;
        let handle = tokio::spawn(async move {
            tokio::time::sleep(interval).await;
            if let Some(enforcer) = enforcer.upgrade() {
                enforcer.on_notify_tick(app_id).await;
            }
        });

        self.notify_timers.insert(app_id, NotifyTimerState { handle, ..state });
    }

    fn cancel_notify_timer(&mut self, app_id: &AppId) {
        if let Some(timer) = self.notify_timers.remove(app_id) {
            timer.handle.abort();
        }
    }

    async fn on_notify_tick(&mut self, app_id: AppId) {
        // Only act if this app is still the currently focused window
        if self.active_window.as_ref().map(|w| w.app_id()) != Some(&app_id) {
            return;  // stale timer — user already switched away
        }

        // Notify again
        let body = format!("You've been using this app for {} minutes...", ...);
        self.platform.notify("Limit reached", &body).await.ok();

        // Advance last_notified_usage and restart timer
        if let Some(mut timer) = self.notify_timers.get_mut(&app_id) {
            timer.last_notified_usage += timer.repeat_interval;
            let state = NotifyTimerState {
                handle: self.spawn_notify_handle(app_id.clone(), timer.repeat_interval),
                ..*timer
            };
            self.notify_timers.insert(app_id.clone(), state);
        }
    }

    /// Shared logic: called on focus switch to handle Notify verdict
    async fn on_notify_verdict(&mut self, app_id: AppId, verdict: &PolicyVerdict::Notify) {
        // Cancel any stale timer for this app
        self.cancel_notify_timer(&app_id);

        // Send immediate notification
        self.platform.notify("Limit reached",
            &format!("You've used {} for {}.", app_id, verdict.state.used))
            .await.ok();

        // Start repeat timer if configured
        if let Some(repeat) = verdict.repeat_interval {
            if repeat > Duration::ZERO {
                let used = verdict.state.used;
                let limit = verdict.state.limit;
                let delay = repeat - Duration::from_secs(
                    ((used - limit).as_secs() % repeat.as_secs()));
                let delay = if delay <= Duration::ZERO { repeat } else { delay };

                let timer_state = NotifyTimerState {
                    policy_id: verdict.policy_id,
                    repeat_interval: repeat,
                    last_notified_usage: used + delay,  // usage at next boundary
                    handle: self.spawn_notify_handle(app_id.clone(), delay),
                };
                self.notify_timers.insert(app_id, timer_state);
            }
        }
    }

    fn spawn_notify_handle(&self, app_id: AppId, delay: Duration) -> JoinHandle<()> {
        let enforcer = self.weak_ref();
        tokio::spawn(async move {
            tokio::time::sleep(delay).await;
            if let Some(enforcer) = enforcer.upgrade() {
                enforcer.on_notify_tick(app_id).await;
            }
        })
    }
}
```

### Initial Delay Calculation

On the first notification (at focus time), the timer delay is the time until the
_next_ boundary:

```
initial_delay = repeat_interval - ((total_seconds - limit) % repeat_interval)
```

If `total_seconds - limit` is exactly a multiple of `repeat_interval`, the
modulo is 0 and `initial_delay = repeat_interval` — meaning the user just
crossed a boundary, so we wait a full interval for the next one.

After that, each timer fires every `repeat_interval` real seconds, assuming
continuous focus.

---

## Step-by-Step

### Block Enforcement

The EnforcerActor handles the block path after `evaluate()` returns `Block`:

```rust
async fn enforce_block(
    &mut self,
    app_id: &AppId,
    verdict: PolicyVerdict,
    policy_config: &PolicyConfig,
) -> Result<()> {
    let PolicyVerdict::Block { policy_id, ref reason, remaining: _ } = verdict else {
        return Ok(());
    };

    let now = Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();

    // 1. Close the PREVIOUS app's interval (if any) — interval management,
    //    NOT block enforcement. The blocked app never had an interval opened.
    //    Check in-memory focus state (passed from TrackerActor).
    if let Some(prev_focus) = self.focus_state.get(&block_uid) {
        // Insert Unfocused to close the previous interval
        // accumulate_interval() runs in the same transaction
        conn.transaction(|conn| {
            accumulate_interval(conn, block_uid, &NewEvent { ... }, Some(prev_focus))?;

            diesel::insert_into(events::table)
                .values(&NewEvent {
                    event_type: EventType::Unfocused as i32,
                    payload: serde_json::json!({"t": now}),
                })
                .execute(conn)?;

            Ok(())
        }).await?;

        self.focus_state.remove(&block_uid);
    }

    // 2. Cancel any limit timer for this app (stale from prior session)
    self.cancel_limit_timer(app_id);

    // 3. Determine overlay buttons from the blocking policy's kind
    let available_actions = match policy_config.kind {
        PolicyKind::Block => {
            // Unconditional block — no time tracking, no extension possible
            vec![OverlayAction::Close]
        }
        PolicyKind::TimeLimit => {
            let state = app_state(&usage, policy_config);
            if state.can_extend() {
                vec![OverlayAction::Extra, OverlayAction::Close]
            } else {
                vec![OverlayAction::Close]
            }
        }
        PolicyKind::Notify => {
            unreachable!("Notify policy triggered enforce_block")
        }
    };

    // 4. Show overlay — fire-and-forget D-Bus call.
    //    No WindowFocused is written for the blocked app.
    //    The event log contains only the Unfocused (previous interval closure).
    let blocked_since = Utc::now();
    let config = ShowOverlayConfig {
        app_id: app_id.as_ref().to_string(),
        policy_id: policy_id as u64,
        reason: BlockReason::from_verdict(reason) as u32,
        blocked_since: blocked_since.timestamp_millis() as u64,
        available_actions: available_actions.iter().map(|a| *a as u32).collect(),
    };

    self.platform.show_overlay(&config).await?;  // fire-and-forget
    // Overlay is plugin-owned; the daemon holds no block state. The signed
    // token (policy_id + blocked_since + signature) travels out with the
    // Overlay(show) call and back with UserAction — see ../architecture/
    // 04-plugin-ipc.md. The user's later choice is resolved in handle_user_action().

    Ok(())
}
```

**No in-memory block state:**

The overlay is owned by the plugin; the daemon keeps no `BlockState` map. The
signed token (`policy_id` + `blocked_since` + `signature`) issued with
`Overlay(show)` is echoed back in `UserAction`, and the daemon verifies the
signature then re-derives `policy_config` from its own DB by `policy_id` (see
[`../architecture/04-plugin-ipc.md`](../architecture/04-plugin-ipc.md)).

```rust
impl EnforcerActor {
    /// Handle a UserAction signal from the plugin.
    /// Called on the main event loop when the user clicks an overlay button.
    async fn handle_user_action(
        &mut self,
        app_id: AppId,
        action: OverlayAction,
        policy_id: u64,
        blocked_since: u64,
        signature: Vec<u8>,
    ) {
        // Plugin is the window authority on (app_id, action). policy_id is only
        // trusted after verifying the Ed25519 signature over the block token
        // (see ../architecture/05-daemon-auth.md).
        if !self.platform.verify_overlay_token(&app_id, policy_id, blocked_since, &signature) {
            warn!(%app_id, "UserAction signature invalid — ignoring");
            return;
        }
        // Re-derive the policy from the daemon's own DB by id — no in-memory
        // block state. The policy (incl. grant_duration) is authoritative here.
        let Ok(policy) = self.policy_store.get(policy_id).await else {
            warn!(%app_id, policy_id, "UserAction references unknown policy — ignoring");
            return;
        };

        match action {
            OverlayAction::Extra => {
                self.grant_extension(&app_id, &policy).await.ok();
            }
            OverlayAction::Close => {
                // Nothing to do — no interval was opened for this app.
                // The previous app's interval was already closed in enforce_block step 1.
                self.platform.hide_overlay(&app_id).await.ok();
            }
        }
    }
}
```

### Option 1: Grant Extra Time

1. EnforcerActor writes a synthetic `WindowFocused` event with the last known
   PID and window title (from the pre-block tracker state). This opens a new
   focus interval.
2. EnforcerActor sets `extended = 1` in `daily_usage` for the app.
3. EnforcerActor **restarts the limit timer** for the extended limit:
   `remaining = (time_limit + extra_seconds) - total_seconds`.
4. The overlay is dismissed via `Overlay(hide)` D-Bus call or the plugin hides
   it automatically when the user clicks.
5. App continues running. The materialized view's accumulated time now counts
   toward the combined cap
   (`policy_config.time_limit_seconds + policy_config.extra_seconds`). When the
   timer fires, the app will be re-evaluated for a potential second block.

```rust
async fn grant_extension(
    &mut self,
    app_id: &AppId,
    policy_config: &PolicyConfig,
) -> Result<()> {
    let now = Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();

    // Write synthetic WindowFocused to start a new interval
    diesel::insert_into(events::table)
        .values(&NewEvent {
            event_type: EventType::WindowFocused as i32,
            payload: serde_json::json!({"t": now, "a": app_id.as_ref()}),
        })
        .execute(&mut self.conn)
        .await?;

    // Mark extended in materialized view
    diesel::sql_query(
        "UPDATE daily_usage SET extended = 1, updated_at = ? WHERE date = ? AND app_id = ?"
    )
    .bind::<diesel::sql_types::Text, _>(&now)
    .bind::<diesel::sql_types::Text, _>(Utc::now().format("%Y-%m-%d").to_string())
    .bind::<diesel::sql_types::Text, _>(app_id.as_ref())
    .execute(&mut self.conn)
    .await?;

    // Restart limit timer for the extended cap
    let extended_limit = policy_config.time_limit_seconds.unwrap_or(0)
        + policy_config.extra_seconds;
    let total_seconds = self.get_daily_usage(app_id).await.unwrap_or(0);
    let remaining = (extended_limit - total_seconds).max(0) as u64;
    self.start_limit_timer(app_id.clone(), remaining);

    // Dismiss overlay via self.platform.hide_overlay() (maps to Overlay(hide) on wire)
    self.platform.hide_overlay(app_id).await.ok();
    Ok(())
}
```

### Option 2: Close App

No additional DB writes are needed. The previous app's interval was already
closed by the `Unfocused` written in `enforce_block` (step 1), and the blocked
app never had a `WindowFocused` written. The overlay is dismissed via
`Overlay(hide)` by `handle_user_action()`, and the app keeps running — but with
no tracked interval, it generates no tracked time.

---

## Overlay Design

The overlay is drawn directly by a **compositor plugin** that loads into the
compositor's address space. For Hyprland, this is `wellbeing-lockdown.so`; for
KWin, a KWin Effect; for Wayfire, a Wayfire plugin; for GNOME Shell, a JS
extension. All communicate with the daemon over the **daemon's bus** (system bus
in system mode, session bus in session mode) using the same interface.

Unlike a client-side overlay (gpui window, layer-shell, etc.), the plugin
renders the overlay UI _after_ the blocked window finishes rendering — giving
pixel-perfect placement with zero latency.

The plugin runs inside the compositor's process space, so it can:

- Hook the render stage to draw OpenGL primitives over any window
- Trap mouse clicks and keyboard events before they reach the app
- Read window geometry directly from compositor memory
- Communicate with the Rust daemon over the daemon's bus (system/session)

### How the Plugin Renders the Overlay

**Step 1: Hook the render stage**

The plugin registers a callback that fires after the target window has finished
rendering:

```
Compositor draws window → Plugin's post-render hook fires
                              │
                              ▼
                     Draw darkened backdrop
                     (full window size, 75% black)
                              │
                              ▼
                     Draw prompt text centered
                     Draw action buttons as quads + labels
                              │
                              ▼
                     Flush OpenGL → next frame
```

```cpp
// Register in PLUGIN_INIT:
HyprlandAPI::registerCallback(PHANDLE, "renderStage",
    [](void* data, SCallbackInfo& info, std::any data) {
        auto* pWindow = std::any_cast<CWindow*>(data);
        if (!g_pLockManager->isTarget(pWindow)) return;
        if (stage != RENDER_PASS_POST_WINDOW) return;
        g_pLockManager->drawOverlay();
    });
```

**Step 2: Draw the overlay UI with OpenGL primitives**

The plugin uses Hyprland's internal `g_pHyprOpenGL` renderer to draw graphic
primitives directly over the blocked window's framebuffer region:

```cpp
void LockManager::drawOverlay() {
    const auto pos = m_targetWindow->m_vRealPosition.vec();
    const auto size = m_targetWindow->m_vRealSize.vec();

    // 75% opaque black backdrop over the entire window
    g_pHyprOpenGL->renderRect(
        CBox{pos.x, pos.y, size.x, size.y},
        CColor{0.0, 0.0, 0.0, 0.75}
    );

    // Prompt and buttons using Hyprland's text renderer
    const int cx = pos.x + (size.x / 2);
    const int cy = pos.y + (size.y / 2);
    drawText(cx, cy - 40,  "Your daily limit has been reached.",
             CColor{1.0, 1.0, 1.0, 1.0}, 16.0f);
    drawButton(cx - 100, cy + 20, "+5 Minutes",  ButtonId::Extra);
    drawButton(cx + 20,  cy + 20, "Close App",   ButtonId::Close);
}
```

The plugin stores each button's bounding box for hit-testing on mouse input.

**Real-world reference:** Study `hyprbars` (in `hyprwm/hyprland-plugins`) for
exactly this pattern: extracting window dimensions, drawing custom containers,
rendering text, and handling clickable regions.

### Input Trapping

The plugin hooks into Hyprland's input event bus to prevent the user from
interacting with the blocked application:

```cpp
// Mouse — onMouseClick internally gates per focused app_id (the directed
// query): it hit-tests the active overlay's buttons and returns true only when
// the focused app has an active overlay. No global "is anything locked?" check.
HyprlandAPI::registerCallback(PHANDLE, "mouse",
    [](void* data, SCallbackInfo& info, std::any data) {
        const auto c = g_pInputManager->getMouseCoords();
        if (g_pLockManager->onMouseClick(c.x, c.y))
            info.cancelled = true;   // swallowed (button hit) → app gets nothing
    });

// Keyboard — onKey() returns true only when the focused app_id is blocked,
// so every key is swallowed for that window and passes through otherwise.
HyprlandAPI::registerCallback(PHANDLE, "key",
    [](void* data, SCallbackInfo& info, std::any data) {
        if (g_pLockManager->onKey())
            info.cancelled = true;
    });
```

Mouse hit-testing (directed: gated by the focused app_id; a button hit emits the
user's choice via the callback; `isTarget(windowHandle)` is the
per-window-handle query used to decide whether a click falls inside a blocked
window):

```cpp
auto LockManager::onMouseClick(double x, double y) -> bool {
    // Directed gate: only the focused app's overlay participates.
    if (m_focusedApp.empty() || !m_overlays.contains(m_focusedApp))
        return false;
    const auto& buttons = m_overlays.at(m_focusedApp).buttons;
    for (const auto& btn : buttons) {
        if (withinRect(btn, x, y)) {
            m_userActionCb(m_focusedApp, static_cast<uint32_t>(btn.actionId));
            return true;   // button hit → swallow the click
        }
    }
    // TODO: if (x, y) falls inside the blocked window bounds, swallow so the
    // app never receives the click. Per-window decision uses isTarget(handle).
    return false;
}
```

### Plugin↔Daemon Communication (D-Bus)

The plugin and Rust daemon communicate over the **daemon's bus** (system bus in
system mode, session bus in session mode). The plugin registers itself with the
daemon via **reverse discovery**: at startup it calls
`Daemon.RegisterPlugin(instance_id)`, claiming a **unique** well-known bus name
(e.g. `org.wellbeing.v1.Manager.<uid>.<sess>`) — a D-Bus well-known name is
unique per connection. The daemon learns the caller's real `uid` via
`SO_PEERCRED` and tracks the instance in `PluginRegistry`, watching
`NameOwnerChanged` for `org.wellbeing.v1.Manager.*` to detect connect/disconnect
(see
[04-plugin-ipc.md](../architecture/04-plugin-ipc.md#multi-instance-plugin-support)).

**D-Bus Interface:**

```xml
<node name="/org/wellbeing/Manager">
  <interface name="org.wellbeing.v1.Manager">

    <!-- Overlay command wrapped in a SignedEnvelope the daemon signs with its
         Ed25519 private key; the plugin verifies it against the public key from
         org.wellbeing.v1.Daemon.DaemonPublicKey. See ../architecture/05-daemon-auth.md.
         ShowOverlayCmd { app_id: s, policy_id: t, reason: u, blocked_since: t, available_actions: au, signature: ay }
         HideOverlayCmd { app_id: s } -->
    <method name="Overlay">
      <arg name="envelope" type="v" direction="in"/>
      <arg name="ack" type="b" direction="out"/>
    </method>

    <!-- Emitted when user clicks an overlay button.
         The plugin is the window authority: `app_id` + `action` are its
         assertion. `policy_id` + `blocked_since` + `signature` are the
         daemon-issued, Ed25519-signed token echoed back (see
         ../architecture/05-daemon-auth.md) — the daemon verifies the
         signature before trusting `policy_id`. The daemon consumes
         this on its main event loop. -->
    <signal name="UserAction">
      <arg name="app_id" type="s"/>
      <arg name="action" type="u"/>
      <arg name="policy_id" type="t"/>
      <arg name="blocked_since" type="t"/>
      <arg name="signature" type="ay"/>
    </signal>

    <!-- Current focus state on every focus change. payload: Option<WindowInfo> -->
    <signal name="FocusChanged">
      <arg name="window" type="v"/>
    </signal>

    <!-- User idle state changed. `idle=true` → emit `Idle` (pause interval);
         `idle=false` → emit `Resumed` (unpause). The plugin tracks
         keyboard/mouse/touchpad/video-player activity. -->
    <signal name="ActivityChanged">
      <arg name="idle" type="b"/>
    </signal>

    <!-- Readable property: current session state.
         Returns the SAME FocusVariant as the FocusChanged signal:
           1 = Desktop, 2 = App {app_id, title, pid, uid, overlay_shown}
         The signal is fire-and-forget and does not persist its value, so this
         property is the canonical, queryable source of truth (GUI reads it on
         startup; daemon uses it for crash recovery). -->
    <property name="CurrentSession" type="v" access="read"/>

  </interface>
</node>
```

**Methods:**

| Method       | Effect                                                                                                                                                         |
| ------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `Overlay(v)` | Show/hide overlay, wrapped in a `SignedEnvelope` the daemon signs (see ../architecture/05-daemon-auth.md). The plugin verifies before acting. Fire-and-forget. |

> **Verification requirement:** the plugin MUST verify the `SignedEnvelope`
> (Ed25519 signature over `payload ‖ issued_at`, plus the freshness window)
> before dispatching the overlay. The C++ sketch below omits verification for
> brevity — the canonical, verified handler lives in
> [../architecture/05-daemon-auth.md](../architecture/05-daemon-auth.md) and
> [../architecture/04-plugin-ipc.md](../architecture/04-plugin-ipc.md).

**Signals:**

| Signal                                                                 | Meaning                                                                                                                                                                                                       |
| ---------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `UserAction(app_id, u, policy_id: t, blocked_since: t, signature: ay)` | User clicked an overlay button. `app_id` + `action` are the plugin's window-domain assertion; `policy_id` + `signature` are the echoed, Ed25519-signed token the daemon verifies before trusting `policy_id`. |
| `FocusChanged(v)`                                                      | `Some(WindowInfo{app_id, title, pid, uid, overlay_shown})` or `None`                                                                                                                                          |

**Enum encoding (all integers in variants):**

```text
Overlay variant command:
  show: {app_id: s, policy_id: t, reason: u, blocked_since: t, available_actions: au, signature: ay}
        (signature = Ed25519 over app_id ‖ policy_id ‖ blocked_since ‖ instance_id;
         the plugin echoes policy_id + blocked_since + signature back in UserAction)
  hide: {app_id: s}

FocusChanged:     variant payload = Option<WindowInfo>

WindowInfo {
    app_id: s,
    title: s,
    pid: u,
    uid: u,
    overlay_shown: b,
}

UserAction signal payload:
  app_id: s, action: u, policy_id: t, blocked_since: t, signature: ay
  (the plugin is the authority on app_id + action; policy_id + blocked_since +
   signature are the daemon-issued, Ed25519-signed token echoed back — see
   ../architecture/05-daemon-auth.md)

OverlayAction:     0=Extra  1=Close
BlockReason:       0=AppTimeLimit  1=CategoryTimeLimit  2=AppBlock  3=CategoryBlock
```

The `Overlay(v)` method is fire-and-forget — it returns immediately with a
boolean ack. User's choice arrives separately via the `UserAction` signal, which
the EnforcerActor consumes on its main event loop:

```
Daemon                              Plugin
  │                                    │
  │ Overlay(show)     │
  │───────────────────────────────────>│  renders overlay
  │<── ack: true ─────────────────────│  returns immediately
  │  [daemon continues processing]    │  user clicks [Grant time]
  │                                    │
  │<═══════════════════════════════════│  UserAction("firefox", 0)
  │ INSERT WindowFocused synthetic     │
  │ UPDATE daily_usage SET extended    │
  │ Overlay(hide)        │
  │───────────────────────────────────>│  hides overlay
```

**FocusChanged signal with overlay_shown:**

The plugin includes an `overlay_shown: bool` in the `WindowInfo` payload of
every `FocusChanged` signal. This tells the daemon, on any focus change, whether
a block overlay is currently rendered on that window. The overlay is a
**plugin-owned state** — the daemon keeps no in-memory block state. On a daemon
restart, a window reported with `overlay_shown == true` lets the daemon refresh
the signed token on that already-rendered overlay (see Startup Recovery below);
the user's later click arrives as a `UserAction` carrying `policy_id` and a
verified signature, and the daemon re-derives the policy from `policy_id`.

**ShowOverlayConfig (Overlay show variant payload):**

```rust
/// Payload for the show variant of Overlay(v) command.
/// Sent as the v variant when command discriminator is "show".
/// Wire form of `OverlayConfig` (see ../architecture/02-platform.md) — `blocked_since`
/// is the unix-ms wall-clock time the block started; no geometry, the plugin
/// reads window dimensions directly from compositor memory. `policy_id` is
/// carried so the plugin can echo it back in `UserAction`; the platform layer
/// signs the payload and embeds the Ed25519 `signature` on dispatch.
pub struct ShowOverlayConfig {
    pub app_id: String,
    pub policy_id: u64,
    pub reason: u32,
    pub blocked_since: u64,
    pub available_actions: Vec<u32>,
}
```

The daemon keeps **no in-memory block state** (no `active_blocks` map). The
overlay is owned by the plugin; `policy_id` travels out with the `Overlay(show)`
call and back with `UserAction`, and the platform layer adds the Ed25519
`signature` that the plugin echoes. The daemon verifies the signature and
re-derives `policy_config` from its own DB when the user acts. See the
signed-token contract in
[../architecture/04-plugin-ipc.md](../architecture/04-plugin-ipc.md).

Rust daemon side (zbus):

The `WindowInfo` struct, `UserActionEvent`, and the `#[proxy] trait Manager`
(the zbus proxy for `org.wellbeing.v1.Manager` — `overlay()`, `current_session`
property, `user_action` signal) are defined **once, canonically**, in
[`../architecture/04-plugin-ipc.md`](../architecture/04-plugin-ipc.md) (§Rust
Side). They are not repeated here to avoid a second source of truth.

**C++ plugin side (Hyprland, sdbus-cpp v2):**

```cpp
#include <sdbus-c++/sdbus-c++.h>

// ── Variant tag discriminants ────────────────────────────────────────────
enum class FocusVariantTag : uint32_t {
    Desktop = 1, App = 2,
};
// CurrentSession reuses FocusVariantTag (see above) — no separate SessionStateTag.

// ── Variant encoding helpers ─────────────────────────────────────────────

auto windowInfoToVariant(const std::optional<WindowInfo>& info) -> sdbus::Variant {
    if (!info.has_value())
        return sdbus::Variant{static_cast<uint32_t>(FocusVariantTag::Desktop)};
    return sdbus::Variant{std::tuple{
        static_cast<uint32_t>(FocusVariantTag::App),
        info->appId, info->title, info->pid, info->uid, info->overlayShown,
    }};
}

/// CurrentSession returns the SAME FocusVariant as the FocusChanged signal, so a
/// late-joining client can read identical state from the readable property (the
/// signal is ephemeral). Both call windowInfoToVariant(currentFocus).
auto buildSessionVariant(const std::optional<WindowInfo>& currentFocus,
                          const LockManager& lm) -> sdbus::Variant {
    (void)lm;  // overlay state is reflected via currentFocus.overlay_shown
    return windowInfoToVariant(currentFocus);
}

// ── WellbeingManager — org.wellbeing.v1.Manager interface ────────────────

class WellbeingManager {
    sdbus::IConnection& m_conn;
    std::unique_ptr<sdbus::IObject> m_object;
    LockManager& m_lockManager;

public:
    WellbeingManager(sdbus::IConnection& connection, LockManager& lockManager)
        : m_conn(connection)
        , m_object(sdbus::createObject(connection,
                     sdbus::ObjectPath{"/org/wellbeing/Manager"}))
        , m_lockManager(lockManager)
    {
        // sdbus-c++ v2: addVTable + forInterface
        m_object->addVTable(
            sdbus::registerMethod("Overlay").implementedAs(
                [this](sdbus::Variant envelope) -> bool {
                    return handleOverlay(envelope);
                }),
            sdbus::registerProperty("CurrentSession").withGetter([]() {
                return buildSessionVariant(
                    /* currentFocus from global */,
                    /* lockManager from global */);
            }),
            sdbus::registerSignal("UserAction")
                .withParameters<std::string, uint32_t, uint64_t, uint64_t,
                                std::vector<uint8_t>>(
                    {"app_id", "action", "policy_id",
                     "blocked_since", "signature"}),
            sdbus::registerSignal("FocusChanged")
                .withParameters<sdbus::Variant>({"window"}),
            sdbus::registerSignal("ActivityChanged")
                .withParameters<bool>({"idle"}))
        .forInterface("org.wellbeing.v1.Manager");

        m_lockManager.setUserActionCallback(
            [this](const std::string& appId, uint32_t action) {
                emitUserAction(appId, action);
            });

        registerWithDaemon();
    }

    void registerWithDaemon() {
        auto daemon = sdbus::createProxy(
            m_conn, sdbus::ServiceName{"org.wellbeing.v1.Daemon"},
            sdbus::ObjectPath{"/org/wellbeing/Daemon"});
        daemon->callMethod("RegisterPlugin")
            .onInterface("org.wellbeing.v1.Daemon")
            .withArguments(instanceId());
    }

    void emitUserAction(const std::string& appId, uint32_t action) {
        m_object->emitSignal("UserAction")
            .onInterface("org.wellbeing.v1.Manager")
            .withArguments(appId, action, m_lockManager.activePolicyId(),
                           m_lockManager.blockedSince(),
                           m_lockManager.activeSignature());
    }

    void emitFocusChanged(const std::optional<WindowInfo>& info) {
        m_object->emitSignal("FocusChanged")
            .onInterface("org.wellbeing.v1.Manager")
            .withArguments(windowInfoToVariant(info));
    }

    void emitActivityChanged(bool idle) {
        m_object->emitSignal("ActivityChanged")
            .onInterface("org.wellbeing.v1.Manager")
            .withArguments(idle);
    }

private:
    auto handleOverlay(const sdbus::Variant& envelope) -> bool {
        // Parse SignedEnvelope { payload(v), issued_at(t), signature(ay) }
        sdbus::Variant payload;
        uint64_t issuedAt = 0;
        std::vector<uint8_t> sig;
        try {
            auto env = envelope.get<
                std::tuple<sdbus::Variant, uint64_t, std::vector<uint8_t>>>();
            payload = std::get<0>(env);
            issuedAt = std::get<1>(env);
            sig = std::get<2>(env);
        } catch (const sdbus::Error&) { return false; }

        if (!verifyEnvelope(m_conn, payload, issuedAt, sig))
            return false;

        return tryShowOverlay(payload) || tryHideOverlay(payload);
    }

    auto tryShowOverlay(sdbus::Variant& payload) -> bool {
        try {
            auto show = payload.get<std::tuple<
                std::string, uint64_t, uint32_t, uint64_t,
                std::vector<uint32_t>, std::vector<uint8_t>>>();
            m_lockManager.showOverlay(
                std::get<0>(show), std::get<1>(show), std::get<2>(show),
                std::get<3>(show), std::get<4>(show), std::get<5>(show));
            return true;
        } catch (const sdbus::Error&) { return false; }
    }

    auto tryHideOverlay(sdbus::Variant& payload) -> bool {
        try {
            m_lockManager.hideOverlay(payload.get<std::string>());
            return true;
        } catch (const sdbus::Error&) { return false; }
    }

    static std::string instanceId() { return "..."; }
};

// ── Ed25519 envelope verification (see docs/architecture/05-daemon-auth.md) ──
bool verifyEnvelope(sdbus::IConnection& conn, const sdbus::Variant& payload,
                     uint64_t issuedAt, const std::vector<uint8_t>& sig) {
    // 1. Freshness: reject if |issuedAt - now| > 30s
    // 2. Ed25519: verify signature over (serialized_payload ‖ issued_at_be)
    //    against the daemon's public key (DaemonPublicKey property).
    // See 05-daemon-auth.md for the canonical implementation.
    (void)conn; (void)payload; (void)issuedAt; (void)sig;
    return false;  // FAIL CLOSED until wired
}
```

> **Key differences from the old scaffold:** sdbus-c++ v2 uses `addVTable()`
> with a single `.forInterface()`. The `CurrentSession` property,
> `FocusChanged`, and `ActivityChanged` signals are registered in the vtable.
> The daemon-issued token (`policy_id`, `blocked_since`, `signature`) is owned
> by `LockManager`, not the `WellbeingManager` itself. The overlay handler
> parses the envelope as a `tuple<variant, uint64_t, vector<uint8_t>>` and
> dispatches via try/catch variant probing. See
> `plugins/hyprland/app/src/main.cpp` for the authoritative implementation.

### Overlay Lifecycle

```
WindowFocused { B } → EnforcerActor evaluates → Block verdict
        │
        ▼
┌──────────────────────────────────────────────────────────────┐
│  1. If previous app A has open interval:                     │
│     INSERT Unfocused (closes A)                                │
│     (TrackerActor accumulate_interval() closes A via         │
│      in-memory focus state — interval management,            │
│      NOT block enforcement)                                  │
│  2. Build ShowOverlayConfig from TimeLimitedApp state        │
│  3. Cancel any stale limit timer for B                      │
│  4. platform.show_overlay(config)  ─── fire-and-forget       │
│     → D-Bus Overlay(show) call                     │
│     → plugin renders overlay on next compositor frame        │
│     → daemon continues processing events immediately         │
│  5. Overlay is plugin-owned — daemon stores no block state.    │
│     The signed token (policy_id+signature) travels out with   │
│     Overlay(show) and back with UserAction.                   │
│                                                              │
│  If plugin not connected → Unfocused already written           │
│  (previous A closed), no overlay possible.                   │
│  App B runs unblocked. On next focus event, re-evaluates.    │
│                                                              │
│  NOTE: No WindowFocused is written for B at any point.       │
│  The event log contains only the Unfocused (A's closure).      │
├──────────────────────────────────────────────────────────────┤
│  5. Per-frame (inside compositor):                           │
│     a. Compositor draws the app normally                     │
│     b. Plugin's render hook fires after blocked window       │
│     c. Plugin draws: dark backdrop + buttons + text          │
│     d. Mouse/keyboard events on target → swallowed           │
│                                                              │
│     User sees: app covered by overlay UI                     │
│     User cannot interact with the blocked app                │
├──────────────────────────────────────────────────────────────┤
│  6. User clicks a button:                                    │
│     Plugin calls emitUserAction(appId, action, policyId,     │
│         blockedSince, signature)  — echoes signed token      │
│     → emits UserAction signal on D-Bus                       │
│     → EnforcerActor receives signal on main event loop       │
│     → calls handle_user_action(app_id, action, policy_id,    │
│         blocked_since, signature)                            │
│                                                              │
│     handle_user_action dispatches:                           │
│       Extra (0) → grant_extension(): INSERT WindowFocused,   │
│           UPDATE extended=1, start limit timer for           │
│           extended cap, Overlay(hide)              │
│       Close (1) → Nothing (no interval to close),            │
│           Overlay(hide)                            │
└──────────────────────────────────────────────────────────────┘
```

### Plugin Disconnect Handling

The plugin is the sole control surface for block resolution. If the plugin's bus
name disappears while a block is active, the overlay is gone and the block is
effectively lifted — the app keeps running with no input trapping.

1. The app keeps running (the overlay was the only enforcement mechanism). The
   limit was reached, but without the plugin there is no overlay to stop the
   user.
2. The **dashboard is read-only** regarding block state — it can display that a
   block was active, but cannot grant time or close the app. Only the overlay
   (when the plugin reconnects) can resolve the block.
3. If the plugin reconnects and the app is still focused, the overlay re-appears
   and normal flow resumes. Since `Overlay(v)` is fire-and-forget, re-showing
   does not block the event loop — the daemon simply sends the config and
   continues. User actions arrive via the `UserAction` signal as usual.
4. The daemon subscribes to `NameOwnerChanged` on the daemon's bus for
   `org.wellbeing.v1.Manager`.

```rust
/// Called when the plugin's bus name disappears while a block is active.
/// The blocked app never had a WindowFocused event persisted — no interval
/// to clean up. The block is lifted until the plugin returns.
fn on_plugin_disconnected(&mut self, app_id: &AppId) {
    warn!(%app_id, "Plugin disconnected — overlay gone, block lifted until reconnect");
}

/// Called when the plugin's bus name (re-)appears. Re-evaluate and, if the app
/// is still blocked, re-issue Overlay(show) with a fresh signed token. No
/// active_blocks map to consult — re-derive from the current policy verdict.
fn on_plugin_reconnected(&mut self, app_id: &AppId) {
    info!(%app_id, "Plugin reconnected — re-evaluating block");
    if let Some(verdict) = self.evaluate(app_id).await {
        if let PolicyVerdict::Block { policy_id, reason, .. } = verdict {
            let config = ShowOverlayConfig {
                app_id: app_id.clone(),
                policy_id,
                reason: BlockReason::from_verdict(&reason) as u32,
                blocked_since: Utc::now().timestamp_millis() as u64,
                available_actions: self.available_actions(app_id),
            };
            // Fire-and-forget — daemon does not await user action here
            self.platform.show_overlay(config).await.ok();
        }
    }
}
```

### Startup Recovery — Plugin Signal Reconciliation

If the daemon crashes while an overlay is active, the plugin retains the overlay
(it keeps rendering on the compositor). On restart, the daemon reconciles by
comparing the last event in the DB with the plugin's current `FocusChanged`
signal (which now includes `uid`) — **no `GetActiveOverlays()` call needed**.

```
Daemon starts
    │
    ▼
Read last event from events table
    │
    ▼
Query plugin current focus state via FocusChanged signal payload: Option<WindowInfo>
    │
    ├─ Plugin reports Some(window):
    │   └─ Compare with last DB event:
    │      ├─ Same app_id, no previous block? OK — synced
    │      ├─ Different app_id? INSERT WindowFocused for current window
│      └─ window.overlay_shown == true:
│         └─ This app was blocked pre-crash. Re-issue a fresh signed token
│            on the already-rendered overlay (Overlay(show) with a new
│            blocked_since + signature). The overlay stays up; only the
│            daemon-issued token is refreshed so the user's later click
│            carries a valid signature. Policy is re-derived by id from DB.
    │
    └─ Plugin reports None:
         └─ Last DB event was WindowFocused? INSERT Unfocused (close interval)
            Last DB event was Unfocused? Do nothing (already consistent)
```

This pure event+signal reconciliation replaces the old `active_blocks` table and
`GetActiveOverlays()` method. No persisted block state is needed — the plugin's
`overlay_shown` flag tells the daemon an overlay is up, and the daemon simply
refreshes the signed token on it. The user's eventual `UserAction` carries a
valid signature and the policy is re-derived by id.

**Recovery scenarios (new event model — blocked apps never have events):**

| DB last event | FocusChanged payload               | Action                                                                                                                                                                                                                                                                                                         |
| ------------- | ---------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| WindowFocused | None                               | INSERT Unfocused (close interval)                                                                                                                                                                                                                                                                              |
| WindowFocused | Same app (uid matches last known)  | OK — synced, open interval continues                                                                                                                                                                                                                                                                           |
| WindowFocused | Different app (uid may differ)     | INSERT WindowFocused for current app                                                                                                                                                                                                                                                                           |
| Unfocused     | None                               | OK — no open interval                                                                                                                                                                                                                                                                                          |
| Unfocused     | Some(app)                          | INSERT WindowFocused for current app                                                                                                                                                                                                                                                                           |
| Unfocused     | Some(app, uid, overlay_shown=true) | **App was blocked pre-crash.** Re-issue a fresh signed token on the already-rendered overlay (Overlay(show) with new blocked_since + signature); the overlay stays up. The user's later UserAction carries a valid signature and the policy is re-derived by id. No block state re-adopted into daemon memory. |

The `overlay_shown` flag eliminates the need for a separate state query. If the
plugin reports a window with `overlay_shown: true`, the daemon refreshes the
signed token on the already-rendered overlay so the user's eventual click
carries a valid signature — no `active_blocks` re-adoption.

---

## Overlay Action Model

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayAction {
    /// Grant extra time (only available in Normal regime).
    Extra = 0,
    /// Close app / dismiss (always available).
    Close = 1,
}
```
