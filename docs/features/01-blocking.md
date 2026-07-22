# Blocking Enforcement Design

## Core Principle: User Choice, Not Automatic Action

The system never automatically closes or terminates applications. Instead:

1. When a policy triggers, the EnforcerActor evaluates before any event is
   persisted. If the app is blocked, only the overlay is shown — no
   WindowFocused event is written, so the blocked app never enters the event
   log.
2. If the previous app has an open focus interval, an Unfocused event closes it
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

The enum has two regimes: Normal (within policy time_limit; limit is
policy.time_limit_minutes) and Extended (user already granted extra time; limit
is policy.time_limit_minutes + policy.extra_minutes). It exposes remaining (time
until limit is reached) and can_extend (whether the user can extend time further
— only once, in Normal regime).

Note: PolicyKind::Block (direct block, no time tracking) has no
time_limit_minutes; it blocks unconditionally when active. For that kind the
overlay shows only Close (no Extra button), and no tracked state is constructed.

### TrackedApp — Unified Domain Model

The new domain model unifies both blocking and notify-only tracking. It is an
enum with two variants: TimeLimited(TimeLimitedApp) for hard deadlines with
optional extension (TimeLimit policies), and TimeTracked(TimeTrackedApp) for
tracked usage with notification reminders (Notify policies). It exposes used()
and remaining() that delegate to the inner variant.

Resolving from DB row plus policy config: app_state() maps a PolicyConfig +
usage tuple to the appropriate TrackedApp variant.

- Block -> unreachable (no tracked state needed).
- TimeLimit -> TimeLimitedApp::Normal or Extended based on whether usage has an
  active extension.
- Notify -> TimeTrackedApp with the notify threshold as limit.

### TimeTrackedApp — Notify-Only Tracked State

A simple struct holding used and limit — no state machine. Notification
scheduling is ephemeral via EnforcerActor timers, not persisted. Exposes
remaining() -> (limit - used).max(0) and is_exceeded() -> used >= limit.

### Overlay Action Availability by State

| TrackedApp            | Overlay buttons | Behaviour                                      |
| --------------------- | --------------- | ---------------------------------------------- |
| TimeLimited(Normal)   | Extra (N) Close | N = extra seconds from policy                  |
| TimeLimited(Extended) | Close only      | Already extended, no further extension allowed |
| TimeTracked(...)      | N/A             | Notify policies never show overlays            |

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

WindowFocused for app B arrives from the plugin as a PlatformEvent. The
EnforcerActor acts as gate — evaluates BEFORE any DB write:

1. Resolve B's app_id -> Vec<CategoryId> (app_categories table)
2. Query policies WHERE active AND (app_id = ? OR category_id IN (...))
3. Query B's daily_usage (total_minutes, extended)
4. Call evaluate(B, &policies, elapsed_usage, now) — PURE DOMAIN FN

If PolicyVerdict::Block: a. Check in-memory focus state — if previous app A has
open interval: INSERT Unfocused (closes A's interval) (TrackerActor
accumulate_interval() closes A via in-memory focus state) b. Build
ShowOverlayConfig with reason, policy_id, and available_actions determined by
policy type: Block -> [Close]; TimeLimit -> app_state(usage,
config).can_extend() ? [Extra, Close] : [Close] c. platform.show_overlay(config)
— fire-and-forget D-Bus d. Do NOT write WindowFocused for B (B never enters
event log — no interval to close)

If PolicyVerdict::Notify: a. INSERT Unfocused (closes previous A's interval) b.
INSERT WindowFocused for B (opens B's interval) (trigger accumulates A, opens B)
c. platform.notify("Limit reached", ...) — D-Bus notification d. Start
notification repeat timer if repeat_interval set: delay = repeat_interval -
((used - limit) % repeat_interval) spawn tokio sleep(delay) When timer fires ->
if B still focused, notify again e. Start limit timer for other policies

If PolicyVerdict::Ok: a. INSERT Unfocused (closes previous A's interval) b.
INSERT WindowFocused for B (opens B's interval) (trigger accumulates A, opens B)
c. Calculate remaining time: if Normal(used, limit): rem = limit - used if
Extended(used, limit+extra): rem = (limit+extra) - used Spawn tokio sleep(rem).
When it fires: re-evaluate B; if limit exceeded -> show overlay

Key properties:

- Policy evaluation happens before any event reaches the DB. If blocked, no
  WindowFocused is written at all.
- The Unfocused written during a block closes the previous app's interval (A),
  not the blocked app's (B never had one).
- Timer-based re-triggering: After a non-blocked app gains focus, a tokio sleep
  task fires when the policy limit would be reached. This catches limit expiry
  during continuous single-app use, not just on focus switches.
- Notify is non-blocking: The app's focus interval proceeds normally.
  Notifications are advisory only — delivered via platform.notify() which calls
  org.freedesktop.Notifications over D-Bus.
- If the daemon crashes between writing Unfocused (step a) and showing the
  overlay (step c), no tracked time is lost — the previous interval is already
  closed. On restart, the next focus event re-evaluates naturally.

## Limit Timer

When an app passes policy check and focus is granted (WindowFocused persisted),
the EnforcerActor spawns a tokio sleep task that fires when the policy limit
would be reached. This catches limit expiry during continuous single-app use,
not just on focus switches.

### Timer Calculation

remaining_minutes() computes the remaining time until the policy limit is
reached. It uses the extended flag from daily_usage to determine whether to use
the base limit or the extended limit (base + extra). Returns 0 if the limit is
already exceeded.

### Timer Lifecycle

App gains focus (WindowFocused persisted): EnforcerActor: 1. Calculate remaining
= remaining_minutes(usage, policy) 2. Start timer:
tokio::spawn(sleep(remaining)) 3. Store JoinHandle in HashMap<AppId,
JoinHandle<()>>

When timer fires: EnforcerActor.on_limit_reached(app_id): 1. Check if app is
still focused (compare with active_window) 2. Query current daily_usage 3.
Re-evaluate policy 4. If Block -> enforce_block() 5. If Ok (policy changed) ->
start new timer

User switches to different app: EnforcerActor cancels previous app's timer
(JoinHandle::abort()), removes from HashMap New app gets its own timer

User extends time (grant_extension): Cancel old timer, start new timer with
remaining = (limit + extra) - total_minutes

### Implementation — EnforcerActor

The EnforcerActor maintains two timer maps: limit_timers for active limit timers
per app (TimeLimit policies), and notify_timers for active notification repeat
timers per app (Notify policies). Both are cancelled on focus switch or
extension.

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

App gains focus (WindowFocused persisted), evaluate returned Notify:
EnforcerActor: 1. platform.notify("Limit reached", ...) — immediate
notification 2. Store last_notified_usage = total_minutes 3. If
repeat_interval > 0: delay = repeat_interval - ((total_minutes - limit) %
repeat_interval) if delay <= 0: delay = repeat_interval (past multiple
intervals) Start timer: tokio::spawn(sleep(delay)) Store JoinHandle in
notify_timers map

When timer fires: EnforcerActor.on_notify_tick(app_id): 1. Check if app is still
focused 2. If yes: platform.notify(...) — re-notify last_notified_usage +=
repeat_interval Start new timer: tokio::spawn(sleep(repeat_interval)) 3. If no:
stale timer, discard

User switches to different app: Cancel notify_timer for app_id Cancel
limit_timer for app_id New app re-evaluated on focus

User grants extension (TimeLimit only): Notification timers are Notify-policy
only; Extension only applies to TimeLimit policies.

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
   focus state (passed from TrackerActor). Insert Unfocused to close the
   previous interval. accumulate_interval() runs in the same transaction.
2. Cancel any limit timer for this app (stale from prior session).
3. Determine overlay buttons from the blocking policy's variant:
   - Block -> [Close] only.
   - TimeLimit -> [Extra, Close] if can_extend(), else [Close].
   - Notify -> unreachable (enforce_block is never called for Notify).
4. Show overlay — fire-and-forget D-Bus call. No WindowFocused is written for
   the blocked app. The event log contains only the Unfocused (previous interval
   closure).

No in-memory block state:

The overlay is owned by the plugin; the daemon keeps no BlockState map. The
signed token (policy_id + blocked_since + signature) issued with Overlay(show)
is echoed back in UserAction, and the daemon verifies the signature then
re-derives policy_config from its own DB by policy_id (see
../architecture/04-plugin-ipc.md).

The daemon keeps no in-memory block state. The overlay is owned by the plugin;
policy_id travels out with the Overlay(show) call and back with UserAction, and
the platform layer adds the Ed25519 signature that the plugin echoes. The daemon
verifies the signature and re-derives policy_config from its own DB when the
user acts.

Rust daemon side (zbus): The WindowInfo struct, UserActionEvent, and
the #[proxy] trait Manager (the zbus proxy for org.wellbeing.v1.Manager —
overlay(), current_focus property, user_action signal) are defined once,
canonically, in ../architecture/04-plugin-ipc.md. They are not repeated here to
avoid a second source of truth.

C++ plugin side (Hyprland, sdbus-cpp v2): The plugin exposes
org.wellbeing.v1.Manager on both the system and session buses. The FocusChanged
signal carries a D-Bus variant whose discriminator separates desktop focus from
application focus: a plain U32(0) means no application window is focused, while
a struct with first field U32(1) means an application window is focused. The
CurrentFocus readable property uses the identical variant encoding, allowing
late-joining clients to read the current focus state even when they missed the
ephemeral signal.

The ActivityChanged signal carries a plain u32 where 0 means idle and 1 means
resumed. The UserAction signal carries the application identifier and the action
the user took (extra time grant or close). On startup the plugin registers with
the daemon and discovers the active daemon bus through a four-step resolution.
If the daemon name appears or disappears, the plugin reconnects, re-registers,
and re-synchronizes overlay state so any rendered blocks are updated after a
daemon restart.

The plugin registers methods, properties, and signals under a single D-Bus
interface table. The overlay handler accepts the signed envelope, parses the
payload, timestamp, and Ed25519 signature, and dispatches to show or hide the
overlay. Verification fails closed until the signature wiring is complete. The
canonical implementation is in plugins/hyprland/app/src/main.cpp.

### Option 1: Grant Extra Time

1. EnforcerActor writes a synthetic WindowFocused event with the last known PID
   and window title (from the pre-block tracker state). This opens a new focus
   interval.
2. EnforcerActor sets extended = 1 in daily_usage for the app.
3. EnforcerActor restarts the limit timer for the extended limit: remaining =
   (time_limit + extra_minutes) - total_minutes.
4. The overlay is dismissed via Overlay(hide) D-Bus call or the plugin hides it
   automatically when the user clicks.
5. App continues running. The materialized view's accumulated time now counts
   toward the combined cap (policy_config.time_limit_minutes +
   policy_config.extra_minutes). When the timer fires, the app will be
   re-evaluated for a potential second block.

### Option 2: Close App

No additional DB writes are needed. The previous app's interval was already
closed by the Unfocused written in enforce_block (step 1), and the blocked app
never had a WindowFocused written. The overlay is dismissed via Overlay(hide) by
handle_user_action(), and the app keeps running — but with no tracked interval,
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

D-Bus Interface:

Methods:

| Method     | Effect                                                                                                                    |
| ---------- | ------------------------------------------------------------------------------------------------------------------------- |
| Overlay(v) | Show/hide overlay, wrapped in a SignedEnvelope the daemon signs (see ../architecture/05-daemon-auth.md). Fire-and-forget. |

Verification requirement: the plugin MUST verify the SignedEnvelope (Ed25519
signature over payload || issued_at, plus the freshness window) before
dispatching the overlay. The C++ sketch below omits verification for brevity;
the canonical, verified handler lives in
[../architecture/05-daemon-auth.md](../architecture/05-daemon-auth.md) and
[../architecture/04-plugin-ipc.md](../architecture/04-plugin-ipc.md).

Signals:

| Signal                                                               | Meaning                                                                                                                                                                                             |
| -------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| UserAction(app_id, u, policy_id: t, blocked_since: t, signature: ay) | User clicked an overlay button. app_id + action are the plugin's window-domain assertion; policy_id + signature are the echoed, Ed25519-signed token the daemon verifies before trusting policy_id. |
| FocusChanged(v)                                                      | Some(WindowInfo{app_id, title, pid, uid, overlay_shown}) or None                                                                                                                                    |

Enum encoding (all integers in variants):

Overlay variant command: show: {app_id: s, policy_id: t, reason: u,
blocked_since: t, available_actions: au, signature: ay} (signature = Ed25519
over app_id || policy_id || blocked_since || instance_id; the plugin echoes
policy_id + blocked_since + signature back in UserAction) hide: {app_id: s}

FocusChanged: variant payload = Option<WindowInfo>

WindowInfo { app_id: s, title: s, pid: u, uid: u, overlay_shown: b, }

UserAction signal payload: app_id: s, action: u, policy_id: t, blocked_since: t,
signature: ay (the plugin is the authority on app_id + action; policy_id +
blocked_since + signature are the daemon-issued, Ed25519-signed token echoed
back — see ../architecture/05-daemon-auth.md)

OverlayAction: 0=Extra 1=Close BlockReason: 0=AppTimeLimit 1=CategoryTimeLimit
2=AppBlock 3=CategoryBlock

The Overlay(v) method is fire-and-forget — it returns immediately with a boolean
ack. User's choice arrives separately via the UserAction signal, which the
EnforcerActor consumes on its main event loop:

Daemon Plugin | | Overlay(show) | |----------------------------------> renders
overlay |<-- ack: true ---------------------- returns immediately | [daemon
continues processing] user clicks [Grant time] |
|=================================================== UserAction("firefox", 0) |
INSERT WindowFocused synthetic | UPDATE daily_usage SET extended | Overlay(hide)
|----------------------------------> hides overlay

FocusChanged signal with overlay_shown:

The plugin includes an overlay_shown: bool in the WindowInfo payload of every
FocusChanged signal. This tells the daemon, on any focus change, whether a block
overlay is currently rendered on that window. The overlay is a plugin-owned
state — the daemon keeps no in-memory block state. On a daemon restart, a window
reported with overlay_shown == true lets the daemon refresh the signed token on
that already-rendered overlay (see Startup Recovery below); the user's later
click arrives as a UserAction carrying policy_id and a verified signature, and
the daemon re-derives the policy from policy_id.

ShowOverlayConfig (Overlay show variant payload):

Payload for the show variant of Overlay(v) command. Sent as the v variant when
command discriminator is "show". Wire form of OverlayConfig — blocked_since is
the unix-ms wall-clock time the block started; no geometry, the plugin reads
window dimensions directly from compositor memory. policy_id is carried so the
plugin can echo it back in UserAction; the platform layer signs the payload and
embeds the Ed25519 signature on dispatch.

pub struct ShowOverlayConfig { app_id: String, policy_id: u64, reason: u32,
blocked_since: u64, available_actions: Vec<u32>, }

The daemon keeps no in-memory block state (no active_blocks map). The overlay is
owned by the plugin; policy_id travels out with the Overlay(show) call and back
with UserAction, and the platform layer adds the Ed25519 signature that the
plugin echoes. The daemon verifies the signature and re-derives policy_config
from its own DB when the user acts. See the signed-token contract in
[../architecture/04-plugin-ipc.md](../architecture/04-plugin-ipc.md).

Rust daemon side (zbus): The WindowInfo struct, UserActionEvent, and
the #[proxy] trait Manager (the zbus proxy for org.wellbeing.v1.Manager —
overlay(), current_focus property, user_action signal) are defined once,
canonically, in ../architecture/04-plugin-ipc.md. They are not repeated here to
avoid a second source of truth.

### Overlay Lifecycle

WindowFocused for B -> EnforcerActor evaluates -> Block verdict | v

1. If previous app A has open interval: INSERT Unfocused (closes A)
   (TrackerActor accumulate_interval() closes A via in-memory focus state —
   interval management, NOT block enforcement)
2. Build ShowOverlayConfig from TimeLimitedApp state
3. Cancel any stale limit timer for B
4. platform.show_overlay(config) — fire-and-forget -> D-Bus Overlay(show) call
   -> plugin renders overlay on next compositor frame -> daemon continues
   processing events immediately
5. Overlay is plugin-owned — daemon stores no block state. The signed token
   (policy_id+signature) travels out with Overlay(show) and back with
   UserAction.

If plugin not connected -> Unfocused already written (previous A closed), no
overlay possible. App B runs unblocked. On next focus event, re-evaluates.

NOTE: No WindowFocused is written for B at any point. The event log contains
only the Unfocused (A's closure).

5. Per-frame (inside compositor): a. Compositor draws the app normally b.
   Plugin's render hook fires after blocked window c. Plugin draws: dark
   backdrop + buttons + text d. Mouse/keyboard events on target -> swallowed

   User sees: app covered by overlay UI User cannot interact with the blocked
   app

6. User clicks a button: Plugin calls emitUserAction(appId, action, policyId,
   blockedSince, signature) — echoes signed token -> emits UserAction signal on
   D-Bus -> EnforcerActor receives signal on main event loop -> calls
   handle_user_action(app_id, action, policy_id, blocked_since, signature)

   handle_user_action dispatches: Extra (0) -> grant_extension(): INSERT
   WindowFocused, UPDATE extended=1, start limit timer for extended cap,
   Overlay(hide) Close (1) -> Nothing (no interval to close), Overlay(hide)

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
   and normal flow resumes. Since Overlay(v) is fire-and-forget, re-showing does
   not block the event loop — the daemon simply sends the config and continues.
   User actions arrive via the UserAction signal as usual.
4. The daemon subscribes to NameOwnerChanged on the daemon's bus for
   org.wellbeing.v1.Manager.

Called when the plugin's bus name disappears while a block is active. The
blocked app never had a WindowFocused event persisted — no interval to clean up.
The block is lifted until the plugin returns.

Called when the plugin's bus name (re-)appears. Re-evaluate and, if the app is
still blocked, re-issue Overlay(show) with a fresh signed token. No
active_blocks map to consult — re-derive from the current policy verdict.

### Startup Recovery — Plugin Signal Reconciliation

If the daemon crashes while an overlay is active, the plugin retains the overlay
(it keeps rendering on the compositor). On restart, the daemon reconciles by
comparing the last event in the DB with the plugin's current FocusChanged
