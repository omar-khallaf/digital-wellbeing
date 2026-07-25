# Blocking Enforcement Design

## Core Principle: User Choice, Not Automatic Action

The system never automatically closes or terminates applications. Instead:

1. When a policy triggers, the EnforcerActor evaluates before any event is
   persisted. If the app is blocked, only the overlay is shown — no Focus event
   is written, so the blocked app never enters the event log.
2. If the previous app has an open focus interval, an Unfocus event closes it
   (this is interval management, not block enforcement).
3. The overlay presents options to the user.
4. The user's choice determines the next action.

Enforcement is overlay-only. The blocked app continues running but the overlay
traps all input, making it impossible to interact with the window. This keeps
the compositor path simple (no process signal handling) and eliminates the need
for capability probing (CAP_SYS_PTRACE) and crash recovery of process state.

## TimeLimitedApp — Domain Model

The EnforcerActor constructs this per-policy after receiving a Block verdict to
determine overlay options for the blocking policy. It is a domain enum
representing the time-limited state of an app against a single policy. It is
constructed from daily_usage plus policy config.

The enum tracks a single regime with limit = policy.time_limit_minutes. It
exposes remaining (time until limit is reached).

Note: PolicyKind::Block (direct block, no time tracking) has no
time_limit_minutes; it blocks unconditionally when active. For that kind the
overlay shows only Close, and no tracked state is constructed.

### TrackedApp — Unified Domain Model

The new domain model unifies both blocking and notify-only tracking. It is an
enum with two variants: TimeLimited(TimeLimitedApp) for hard deadlines with
optional extension (TimeLimit policies), and TimeTracked(TimeTrackedApp) for
tracked usage with notification reminders (Notify policies). It exposes used()
and remaining() that delegate to the inner variant.

Resolving from DB row plus policy config: app_state() maps a PolicyConfig +
usage tuple to the appropriate TrackedApp variant.

- Block -> unreachable (no tracked state needed).
- TimeLimit -> TimeLimitedApp.
- Notify -> TimeTrackedApp with the notify threshold as limit.

### TimeTrackedApp — Notify-Only Tracked State

A simple struct holding used and limit — no state machine. Notification
scheduling is ephemeral via EnforcerActor timers, not persisted. Exposes
remaining() -> (limit - used).max(0) and is_exceeded() -> used >= limit.

### Overlay Action Availability by State

| TrackedApp       | Overlay buttons | Behaviour                           |
| ---------------- | --------------- | ----------------------------------- |
| TimeLimited(...) | Close only      | Close dismisses the overlay         |
| TimeTracked(...) | N/A             | Notify policies never show overlays |

## Blocking Flow

### Policy Evaluation — Pure Domain Function

Evaluates ALL policies relevant to an app with AND semantics:

- Block wins over everything — if ANY policy blocks, the app is blocked.
- Notify verdicts stack as advisory only (first Notify determines payload).
- The first blocking policy determines the overlay reason.

The function accepts app_id, a pre-filtered policy slice (by data layer, using
app_id + categories), elapsed_usage (total_minutes from daily_usage), and now
(explicit — no Clock dependency). It returns PolicyVerdict.

Filtering (data layer, before evaluate() is called): The EnforcerActor resolves
the app's categories first (via app_categories table), then queries only
matching policies. The domain function never loads all policies.

AND semantics: The function iterates all matching policies:

- PolicyKind::Block -> immediate PolicyVerdict::Block (unconditional, no time
  tracking)
- PolicyKind::TimeLimit, remaining <= 0 -> PolicyVerdict::Block
- PolicyKind::Notify, remaining <= 0 -> first Notify triggers
  PolicyVerdict::Notify; subsequent Notify violations are collected but don't
  override the first (Block still wins)
- All pass -> PolicyVerdict::Ok
- Notify triggered but no Block -> PolicyVerdict::Notify

### Full Flow

Focus for app B arrives from the plugin as a PlatformEvent. The EnforcerActor
acts as gate — evaluates BEFORE any DB write:

1. Resolve B's app_id -> Vec<CategoryId> (app_categories table)
2. Query policies WHERE active AND (app_id = ? OR category_id IN (...))
3. Query B's daily_usage (total_minutes)
4. Call evaluate(B, &policies, elapsed_usage, now) — PURE DOMAIN FN

If PolicyVerdict::Block: a. Check in-memory focus state — if previous app A has
open interval: INSERT Unfocus (closes A's interval) (EnforcerActor
`accumulate_daily_usage` closes A via in-memory focus state) b. Build
ShowOverlayConfig with reason, policy_id, and available_actions: Block ->
[Close]; TimeLimit -> [Close] c. platform.show_overlay(config) — fire-and-forget
D-Bus d. Do NOT write Focus for B (B never enters event log — no interval to
close)

If PolicyVerdict::Notify: a. INSERT Unfocus (closes previous A's interval) b.
INSERT Focus for B (opens B's interval) (trigger accumulates A, opens B) c.
platform.notify("Limit reached", ...) — D-Bus notification d. Start notification
repeat timer if repeat_interval set: delay = repeat_interval - ((used - limit) %
repeat_interval) spawn tokio sleep(delay) When timer fires -> if B still
focused, notify again e. Start limit timer for other policies

If PolicyVerdict::Ok: a. INSERT Unfocus (closes previous A's interval) b. INSERT
Focus for B (opens B's interval) (trigger accumulates A, opens B) c. Calculate
remaining time: rem = limit - used. Spawn tokio sleep(rem). When it fires:
re-evaluate B; if limit exceeded -> show overlay

Key properties:

- Policy evaluation happens before any event reaches the DB. If blocked, no
  Focus is written at all.
- The Unfocus written during a block closes the previous app's interval (A), not
  the blocked app's (B never had one).
- Timer-based re-triggering: After a non-blocked app gains focus, a tokio sleep
  task fires when the policy limit would be reached. This catches limit expiry
  during continuous single-app use, not just on focus switches.
- Notify is non-blocking: The app's focus interval proceeds normally.
  Notifications are advisory only — delivered via platform.notify() which calls
  org.freedesktop.Notifications over D-Bus.
- If the daemon crashes between writing Unfocus (step a) and showing the overlay
  (step c), no tracked time is lost — the previous interval is already closed.
  On restart, the next focus event re-evaluates naturally.

## Limit Timer

When an app passes policy check and focus is granted (Focus persisted), the
EnforcerActor spawns a tokio sleep task that fires when the policy limit would
be reached. This catches limit expiry during continuous single-app use, not just
on focus switches.

### Timer Calculation

remaining_minutes() computes the remaining time until the policy limit is
reached: remaining = max(0, limit - used). Returns 0 if the limit is already
exceeded.

### Timer Lifecycle

App gains focus (Focus persisted): EnforcerActor: 1. Calculate remaining =
remaining_minutes(usage, policy) 2. Start timer:
tokio::spawn(sleep(remaining)) 3. Store JoinHandle in HashMap<AppId,
JoinHandle<()>>

When timer fires: EnforcerActor.on_limit_reached(app_id): 1. Check if app is
still focused (compare with active_window) 2. Query current daily_usage 3.
Re-evaluate policy 4. If Block -> enforce_block() 5. If Ok (policy changed) ->
start new timer

User switches to different app: EnforcerActor cancels previous app's timer
(JoinHandle::abort()), removes from HashMap New app gets its own timer

Block resolves (user closes overlay): Cancel limit timer for the app.

### Implementation — EnforcerActor

The EnforcerActor maintains two timer maps: limit_timers for active limit timers
per app (TimeLimit policies), and notify_timers for active notification repeat
timers per app (Notify policies). Both are cancelled on focus switch.

The actor uses a weak reference pattern to avoid holding a strong reference
cycle within the actor. The EnforcerActor uses Arc<Mutex<...>> interior
mutability (or an mpsc channel back to itself) to safely access actor state from
the spawned timer task.

Limit timer methods:

- start_limit_timer(app_id, remaining_secs): cancels any existing timer for the
  app, spawns a tokio sleep that calls on_limit_reached on fire.
- cancel_limit_timer(app_id): removes and aborts the existing handle.
- on_limit_reached(app_id): checks if app is still focused, re-queries usage and
  policies, re-evaluates, and enforces block if needed.

Notify timer methods:

- start_notify_timer(app_id, state): cancels existing timer, spawns a tokio
  sleep that calls on_notify_tick on fire.
- cancel_notify_timer(app_id): removes and aborts.
- on_notify_tick(app_id): checks if app is still focused, sends a desktop
  notification, advances last_notified_usage by repeat_interval, restarts the
  timer.

Notification timer methods share spawn_notify_handle(), which creates the
tokio::spawn(sleep(delay)) weak-reference pattern.

## Notification Timer (Notify Policies)

When a Notify policy triggers and notification_repeat_interval_minutes is set,
the EnforcerActor starts a real-time timer that fires at the repeat interval
while the app remains focused. This catches the case where the user keeps using
the app past the limit; they get periodic reminders.

### Timer Calculation

The timer delay aligns to the next notification boundary based on the usage
known at the last focus event:

delay = repeat_interval - ((total_minutes - limit) % repeat_interval)

Example: limit=1h (3600s), repeat=5min (300s), usage at focus=3720s (1h2min) ->
delay = 300 - ((3720 - 3600) % 300) = 300 - (120 % 300) = 300 - 120 = 180s

The timer fires after 180 real seconds. If the app is still focused at that
point, the usage has accumulated to >= 3900s (1h5min) and a new notification is
sent.

### Timer Lifecycle

App gains focus (Focus persisted), evaluate returned Notify: EnforcerActor: 1.
platform.notify("Limit reached", ...) — immediate notification 2. Store
last_notified_usage = total_minutes 3. If repeat_interval > 0: delay =
repeat_interval - ((total_minutes - limit) % repeat_interval) if delay <= 0:
delay = repeat_interval (past multiple intervals) Start timer:
tokio::spawn(sleep(delay)) Store JoinHandle in notify_timers map

When timer fires: EnforcerActor.on_notify_tick(app_id): 1. Check if app is still
focused 2. If yes: platform.notify(...) — re-notify last_notified_usage +=
repeat_interval Start new timer: tokio::spawn(sleep(repeat_interval)) 3. If no:
stale timer, discard

User switches to different app: Cancel notify_timer for app_id Cancel
limit_timer for app_id New app re-evaluated on focus

Close resolves the block (TimeLimit only): The overlay is dismissed and the
limit timer is cancelled.

### Initial Delay Calculation

On the first notification (at focus time), the timer delay is the time until the
next boundary:

initial_delay = repeat_interval - ((total_minutes - limit) % repeat_interval)

If total_minutes - limit is exactly a multiple of repeat_interval, the modulo is
0 and initial_delay = repeat_interval — meaning the user just crossed a
boundary, so we wait a full interval for the next one.

After that, each timer fires every repeat_interval real seconds, assuming
continuous focus.

## Block Enforcement

The EnforcerActor handles the block path after evaluate() returns Block:

1. Close the PREVIOUS app's interval (if any) — interval management, NOT block
   enforcement. The blocked app never had an interval opened. Check in-memory
   focus state (passed from EnforcerActor). Insert Unfocus to close the previous
   interval. `accumulate_daily_usage` runs in the same transaction.
2. Cancel any limit timer for this app (stale from prior session).
3. Overlay shows Close button for all block types.
4. Show overlay — fire-and-forget D-Bus call. No Focus is written for the
   blocked app. The event log contains only the Unfocus (previous interval
   closure).

No in-memory block state:

The overlay is owned by the plugin; the daemon keeps no active overlay map.
Block state is shared declaratively through the ActiveBlocks D-Bus property. The
plugin reads state from ActiveBlocks and renders overlays accordingly. Close is
handled locally in the plugin via LockManager::hideOverlay() -- no signal is
sent to the daemon.

Rust daemon side (zbus): The WindowInfo struct and the #[proxy] trait Manager
(the zbus proxy for org.wellbeing.v1.Manager — current_focus property) are
defined once, canonically, in ../architecture/04-plugin-ipc.md. They are not
repeated here to avoid a second source of truth.

C++ plugin side (Hyprland, sdbus-cpp v2): The plugin exposes
org.wellbeing.v1.Manager on both the system and session buses. The unified Event
signal carries a (u32, u32, String, String, u32, u32) struct whose first u32
discriminator separates desktop focus from application focus, with distinct tags
for Focus (tag=1) and Block (tag=2). A plain U32(0) means no application window
is focused. The CurrentFocus readable property uses the identical tag encoding,
allowing late-joining clients to read the current focus state even when they
missed the ephemeral signal.

On startup the plugin registers with the daemon and discovers the active daemon
bus through a four-step resolution. If the daemon name appears or disappears,
the plugin reconnects, re-registers, and re-synchronizes overlay state so any
rendered blocks are updated after a daemon restart.

The canonical implementation is in plugins/hyprland/app/src/main.cpp.

### Close App

No additional DB writes are needed. The previous app's interval was already
closed by the Unfocus written in enforce_block (step 1), and the blocked app
never had a Focus written. The close button is handled locally in the plugin via
LockManager::hideOverlay(), and the app keeps running with no tracked interval —
it generates no tracked time.

## Overlay Design

The overlay is drawn directly by a compositor plugin that loads into the
compositor's address space. For Hyprland, this is wellbeing-lockdown.so; for
KWin, a KWin Effect; for Wayfire, a Wayfire plugin; for GNOME Shell, a JS
extension. All communicate with the daemon over the daemon's bus (system bus in
system mode, session bus in session mode) using the same interface.

Unlike a client-side overlay (gpui window, layer-shell, etc.), the plugin
renders the overlay UI after the blocked window finishes rendering — giving
pixel-perfect placement with zero latency.

The plugin runs inside the compositor's process space, so it can:

- Hook the render stage to draw OpenGL primitives over any window
- Trap mouse clicks and keyboard events before they reach the app
- Read window geometry directly from compositor memory
- Communicate with the Rust daemon over the daemon's bus (system/session)

### How the Plugin Renders the Overlay

Step 1: Hook the render stage

The plugin registers a callback that fires after the target window has finished
rendering:

Compositor draws window -> Plugin's post-render hook fires | v Draw darkened
backdrop (full window size, 75% black) | v Draw prompt text centered Draw action
buttons as quads + labels | v Flush OpenGL -> next frame

Step 2: Draw the overlay UI with OpenGL primitives

The plugin uses the compositor's internal OpenGL renderer to draw graphic
primitives directly over the blocked window's framebuffer region. It renders a
75% opaque black backdrop over the entire window, then draws prompt and button
text centered. The plugin stores each button's bounding box for hit-testing on
mouse input.

Real-world reference: Study hyprbars (in hyprwm/hyprland-plugins) for exactly
this pattern: extracting window dimensions, drawing custom containers, rendering
text, and handling clickable regions.

### Input Trapping

The plugin hooks into the compositor's input event bus to prevent the user from
interacting with the blocked application:

Mouse — onMouseClick internally gates per focused app_id (the directed query):
it hit-tests the active overlay's buttons and returns true only when the focused
app has an active overlay. No global "is anything locked?" check.

Keyboard — onKey() returns true only when the focused app_id is blocked, so
every key is swallowed for that window and passes through otherwise.

Mouse hit-testing (directed: gated by the focused app_id; a button hit emits the
user's choice via the callback; isTarget(windowHandle) is the per-window-handle
query used to decide whether a click falls inside a blocked window):

LockManager::onMouseClick(x, y) -> bool: Directed gate: only the focused app's
overlay participates. if focused app is empty or not in overlays: return false.
For each button in the focused app's overlays: If click is within button rect:
emit user action with app and button action return true (button hit -> swallow
the click) If click falls inside the blocked window bounds: swallow so the app
never receives the click. Per-window decision uses isTarget(handle). Otherwise
return false.

### Plugin↔Daemon Communication (D-Bus)

The plugin and Rust daemon communicate over the daemon's bus (system bus in
system mode, session bus in session mode). The plugin registers itself with the
daemon via reverse discovery: at startup it calls Controller.RegisterPlugin().
The plugin connects anonymously and the bus daemon assigns it a unique bus name
(:1.xxx). The daemon learns the caller's real uid via SO_PEERCRED and unique bus
name (from header.sender()), and tracks the instance in PluginRegistry, watching
the plugin's connection for connect/disconnect. (see
[04-plugin-ipc.md](../architecture/04-plugin-ipc.md#multi-instance-plugin-support)).

D-Bus Interface (org.wellbeing.v1.Manager):

The plugin exposes signals and a property, but no methods for the daemon to
call. The daemon never commands the plugin — block state is shared declaratively
through the daemon's ActiveBlocks property and BlockedAppsChanged signal (see
[04-plugin-ipc.md](../architecture/04-plugin-ipc.md)).

Signals (plugin -> daemon):

| Signal | Payload                                                                                                                 |
| ------ | ----------------------------------------------------------------------------------------------------------------------- |
| Event  | (u32, u32, String, String, u32, u32) — (tag, variant, app_id, title, pid, uid); tag=0=Desktop, tag=1=Focus, tag=2=Block |

Property:

| Property     | Type | Returns                                                                           |
| ------------ | ---- | --------------------------------------------------------------------------------- |
| CurrentFocus | v    | Same tag encoding as Event signal — Desktop (tag=0), Focus (tag=1), Block (tag=2) |

The Block variant (tag=2) is used when the focused window has an active overlay.
The plugin determines this by checking LockManager::isOverlayShown() at emit
time. The variant tag encodes the distinction without a separate boolean field.

Blocking state is published by the daemon on Controller:

| Property / Signal  | Type | Purpose                                     |
| ------------------ | ---- | ------------------------------------------- |
| ActiveBlocks       | a(u) | Readable list of all currently blocked apps |
| BlockedAppsChanged | s    | Signal: {app_id, blocked: bool}             |

The daemon writes to ActiveBlocks when a block starts or ends. The plugin reads
ActiveBlocks on startup and subscribes to BlockedAppsChanged for live updates.
Overlay rendering is triggered by the plugin's local overlay set, not by daemon
commands.

Close button handling is entirely local to the plugin: the plugin calls
LockManager::hideOverlay() to dismiss the overlay without any D-Bus signal.

### Overlay Lifecycle

Focus for B -> EnforcerActor evaluates -> Block verdict | v

1. If previous app A has open interval: INSERT Unfocus (closes A) (EnforcerActor
   `accumulate_daily_usage` closes A via in-memory focus state — interval
   management, NOT block enforcement)
2. Daemon adds app to ActiveBlocks -> emits BlockedAppsChanged signal on D-Bus
3. Cancel any stale limit timer for B
4. Plugin (subscribed to BlockedAppsChanged) receives the signal, reads
   ActiveBlocks for full block details, and renders overlay on next compositor
   frame -> daemon continues processing events immediately
5. Overlay is plugin-owned — daemon stores no block state. Block state is shared
   declaratively through ActiveBlocks.

If plugin not connected -> Unfocus already written (previous A closed), no
overlay possible. App B runs unblocked. On next focus event, re-evaluates.

NOTE: No Focus is written for B at any point. The event log contains only the
Unfocus (A's closure).

5. Per-frame (inside compositor): a. Compositor draws the app normally b.
   Plugin's render hook fires after blocked window c. Plugin draws: dark
   backdrop + buttons + text d. Mouse/keyboard events on target -> swallowed

   User sees: app covered by overlay UI User cannot interact with the blocked
   app

6. User clicks Close: Plugin calls LockManager::hideOverlay() locally. No D-Bus
   signal is sent — the overlay is dismissed locally in the plugin. The app
   continues running with no tracked interval.

### Plugin Disconnect Handling

The plugin is the sole control surface for block resolution. If the plugin's bus
name disappears while a block is active, the overlay is gone and the block is
effectively lifted — the app keeps running with no input trapping.

1. The app keeps running (the overlay was the only enforcement mechanism). The
   limit was reached, but without the plugin there is no overlay to stop the
   user.
2. The dashboard is read-only regarding block state — it can display that a
   block was active, but cannot grant time or close the app. Only the overlay
   (when the plugin reconnects) can resolve the block.
3. If the plugin reconnects and the app is still focused, the overlay re-appears
   and normal flow resumes. The plugin re-reads ActiveBlocks from the daemon and
   re-establishes overlays for all blocked apps. Block resolution is handled
   locally by the plugin.
4. The daemon subscribes to NameOwnerChanged on the daemon's bus for
   org.wellbeing.v1.Manager.

Called when the plugin's bus name disappears while a block is active. The
blocked app never had a Focus event persisted — no interval to clean up. The
block is lifted until the plugin returns.

Called when the plugin's bus name (re-)appears. Re-evaluate and, if the app is
still blocked, update ActiveBlocks — the plugin re-reads the property and
re-establishes overlays. No active_blocks map to consult — re-derive from the
current policy verdict.

### Startup Recovery — Plugin State Reconciliation

If the daemon crashes while an overlay is active, the plugin retains the overlay
(it keeps rendering on the compositor). On restart, the daemon reconciles by
comparing the last event in the DB with the plugin's current focus state via
CurrentFocus
