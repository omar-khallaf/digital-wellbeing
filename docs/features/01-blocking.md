# Blocking Enforcement Design

## Core Principle: User Choice, Not Automatic Action

The system never automatically closes or terminates applications. Instead:

1. Policy evaluation runs on **every focus switch** (prevents evasion: the user
   cannot escape by switching away before a timer fires) AND on a **per-minute
   tick** for the currently focused app (catches TimeLimit expiry during
   continuous use).
2. The event log is a true append-only record — Focus events are always written.
   If the app is blocked, a Blocked event terminates the interval. No synthetic
   Unfocus events.
3. The overlay displays a Close Window button. The user cannot dismiss the
   overlay — locked mode is the default. The only action is terminating the
   blocked window.
4. The user clicks Close Window → the plugin terminates the window via the
   compositor API. The block remains in effect: if the user re-launches the app,
   the overlay re-appears immediately. The block is lifted only by editing the
   policy.

Enforcement is overlay-only. The blocked app continues running but the overlay
traps all input, making it impossible to interact with the window. This keeps
the compositor path simple (no process signal handling) and eliminates the need
for capability probing (CAP_SYS_PTRACE) and crash recovery of process state.

### Policy Evaluation — Pure Domain Function

Evaluates ALL matching policies sorted by priority. Returns the first
terminating effect (`Allow`/`Block`/`TimeLimit`) plus any `Notify` effects
encountered before it. No matching policies → `None` (unrestricted).

The function accepts a pre-filtered, pre-sorted policy slice, elapsed_usage
minutes, and `now`. It is pure — no I/O, no clock dependency.

Iteration logic:

```
for policy in policies:
    match policy.effect:
        Allow    → return Allow (terminating)
        Block    → return Block (terminating)
        TimeLimit(n) if used >= n → return TimeLimit (terminating)
        TimeLimit(n) → continue (not exceeded yet, but track it)
        Notify(n) if used (rounded minute) == n → register notification (non-terminating), continue
        Notify(n) → continue
no terminating match → None (unrestricted)
```

### Full Blocking Flow

Two evaluation paths, both using the same pure `evaluate()` function:

**Path 1 — Per-focus switch (always, prevents evasion):**

Focus for app B arrives from the plugin. The EnforcerActor:

1. Upsert app: `INSERT INTO apps (app_class) VALUES (?) ON CONFLICT DO NOTHING`.
2. The plugin sends an Event signal with **either** `tag=0` (Focus) or `tag=2`
   (Block) for app B, depending on whether the plugin knows B is blocked:
   - If plugin sees B in `BlockedApps` → sends `Event(tag=2, app="B")`.
   - Otherwise → sends `Event(tag=0, app="B")`.
3. Daemon writes the event to DB as-is (event_type 0 for Focus, 8 for Blocked).
   No synthetic events, no gate. A Focus event naturally terminates the previous
   app A's interval. A Blocked event terminates B's interval without
   accumulating time.
4. If the event was Focus: resolve B's app_class → `Vec<CategoryId>`, query
   policies + usage → `evaluate(B, &policies, usage, now)`.
   - If Block/TimeLimit exceeded → update `BlockedApps` D-Bus property. Plugin
     reads BlockedAppsChanged → knows B is blocked → shows overlay. On next
     focus for B, plugin sends Block(tag=2) instead of Focus.
   - If Notify → `platform.notify()` (one-shot).
   - If Allow/unrestricted → no action.

**Path 2 — Per-minute tick for focused apps (eliminates per-app timers):**

Hooks into the existing minute-ticker (aligned to wall-clock minute boundaries,
same tick that drives buffer flushes in buffer.rs). Tick order:

1. Flush buffered events to DB (batch INSERT).
2. Run `accumulate_daily_usage` to update `daily_usage_by_app`,
   `daily_usage_by_category`, and `daily_usage_by_title` projections.
3. Read the single currently focused app from plugin state, query its usage +
   policies, call `evaluate()`.

If the verdict changes from Allow to Block (TimeLimit expired during continuous
use), updates `BlockedApps` — the plugin reacts, shows overlay, and switches to
sending Block(tag=2). If the verdict clears (policy edited while blocked),
updates `BlockedApps` — plugin removes overlay and switches back to
Focus(tag=0).

**Event type decision (plugin-side):**

```
if app_class in daemon.BlockedApps:
    send Event(tag=2=Block, app=app_class, ...)
else:
    send Event(tag=0=Focus, app=app_class, ...)
```

The daemon never commands the plugin. It publishes block state via `BlockedApps`
/ `BlockedAppsChanged`. The plugin reads that state and decides which event tag
to emit.

**Verdict handling (daemon-side, same for both paths):**

| Verdict                     | Action                                                                  | Interval                                                               |
| --------------------------- | ----------------------------------------------------------------------- | ---------------------------------------------------------------------- |
| Block or TimeLimit exceeded | Update `BlockedApps` → plugin reads → shows overlay, sends Block events | Blocked event terminates interval; Focus→Blocked span IS counted (ms). |
| Notify exceeded             | `platform.notify()` (one-shot)                                          | Continues normally                                                     |
| Allow / unrestricted        | No action                                                               | Continues normally                                                     |

**Key properties:**

- Daemon never generates synthetic events. Plugin sends Focus or Block; daemon
  writes it as-is.
- First time a blocked app gets focus: plugin sends Focus (doesn't know yet),
  daemon evaluates → Block → updates BlockedApps → plugin learns → switches to
  Block events. The initial Focus→Blocked span counts (ms).
- Subsequent focus attempts on the same blocked app: plugin sends Block
  directly.
- BlockedApps is the single source of truth for block state.

## Block Enforcement

The daemon never commands the plugin. Block state is shared declaratively:

1. Daemon evaluates → Block or TimeLimit exceeded.
2. Daemon updates `BlockedApps` D-Bus property → emits `BlockedAppsChanged`.
3. Plugin (subscribed to `BlockedAppsChanged`) reads signal data, sees the app
   is blocked, renders overlay on next compositor frame.
4. The overlay traps all input (mouse + keyboard) — locked mode, no dismiss.
5. Close Window button terminates the window via compositor API. The block
   persists in `BlockedApps` — if the user re-launches, the plugin sees the app
   is still in `BlockedApps` and sends `Event(tag=2=Block)` instead of Focus.
   The overlay re-appears on the next compositor frame.
6. The block is lifted only by editing the policy. When the daemon removes the
   app from `BlockedApps`, the plugin sees the update and stops rendering the
   overlay.

No in-memory block state in the daemon beyond `BlockedApps`. The plugin owns the
overlay rendering.

### Plugin Event Tag Decision

The plugin determines which tag to send based on `BlockedApps`:

```
on_focus(window_handle):
    app_class = resolve_app_class(window_handle)
    if app_class in daemon.BlockedApps:
        send Event(tag=2=Block, app_class, title, pid, power_tag)
    else:
        send Event(tag=0=Focus, app_class, title, pid, power_tag)
```

After the initial block (first Focus event that triggered the Block verdict),
the daemon adds the app to `BlockedApps`. On the **next** focus for that app
(e.g., user re-launches after Close Window), the plugin sees it in `BlockedApps`
and sends Block(tag=2) directly — no Focus event is written, only a Blocked(8)
event that terminates the interval.

### Close App

When the user clicks Close Window on a blocked app:

1. Plugin terminates the window via the compositor API (e.g.,
   `hyprctl dispatch closewindow` on Hyprland). No D-Bus signal sent.
2. The block remains in `BlockedApps`. If the user re-launches the app, the
   plugin sends `Event(tag=2)` — a Blocked(8) event is written, no Focus opens.
3. The block persists until the user edits or deletes the blocking policy.

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

Mouse — onMouseClick internally gates per focused app_class (the directed
query): it hit-tests the active overlay's buttons and returns true only when the
focused app has an active overlay. No global "is anything locked?" check.

Keyboard — onKey() returns true only when the focused app_class is blocked, so
every key is swallowed for that window and passes through otherwise.

Mouse hit-testing (directed: gated by the focused app_class; a button hit emits
the user's choice via the callback; isTarget(windowHandle) is the
per-window-handle query used to decide whether a click falls inside a blocked
window):

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
through the daemon's BlockedApps property and BlockedAppsChanged signal (see
[04-plugin-ipc.md](../architecture/04-plugin-ipc.md)).

Signals (plugin -> daemon):

| Signal | Payload                                                                                                            |
| ------ | ------------------------------------------------------------------------------------------------------------------ |
| Event  | (u32, String, String, u32, u32) — (tag, app_class, title, pid, power_tag); tag=0=Focus, tag=1=Unfocus, tag=2=Block |

Property:

| Property     | Type | Returns                                                                           |
| ------------ | ---- | --------------------------------------------------------------------------------- |
| CurrentFocus | v    | Same tag encoding as Event signal — Focus (tag=0), Unfocus (tag=1), Block (tag=2) |

The Block variant (tag=2) is used when the focused window has an active overlay.
The plugin determines this by checking LockManager::isOverlayShown() at emit
time. The variant tag encodes the distinction without a separate boolean field.

Blocking state is published by the daemon on Controller:

| Property / Signal  | Type         | Purpose                                     |
| ------------------ | ------------ | ------------------------------------------- |
| BlockedApps        | a(s(tutau))  | Readable list of all currently blocked apps |
| BlockedAppsChanged | (u, s, b, u) | Signal: {uid, app_class, blocked, reason}   |

The daemon writes to BlockedApps when a block starts or ends. The plugin reads
BlockedApps on startup and subscribes to BlockedAppsChanged for live updates.
Overlay rendering is triggered by the plugin's local overlay set, not by daemon
commands.

Close Window is handled by the plugin via the compositor API — it terminates the
blocked window. The block persists in the daemon's BlockedApps state. No D-Bus
signal is sent.

### Overlay Lifecycle

**Plugin startup:**

1. Plugin connects to the daemon, reads the full `BlockedApps` property.
2. If the currently focused window is in `BlockedApps`, plugin shows overlay
   immediately and sends `Event(tag=2=Block)` for the current app.
3. Plugin subscribes to `BlockedAppsChanged` for live updates.

**First focus on a blocked app:**

1. Plugin doesn't know B is blocked yet → sends `Event(tag=0=Focus, app="B")`.
2. Daemon writes Focus(0) for B (terminates previous app A's interval).
3. Daemon evaluates → Block verdict → adds B to `BlockedApps` → emits
   `BlockedAppsChanged`.
4. Plugin receives `BlockedAppsChanged`, reads `BlockedApps`, sees B is blocked
   and the currently focused window is B → **immediately** sends
   `Event(tag=2=Block, app="B")` for the currently focused window (no focus
   switch needed) → daemon writes Blocked(8).
5. Plugin renders overlay on next compositor frame.
6. Per-frame (inside compositor): a. Compositor draws the app normally. b.
   Plugin's render hook fires after blocked window. c. Plugin draws: dark
   backdrop + buttons + text. d. Mouse/keyboard events on target → swallowed.

The Focus→Blocked span (step 1 to the immediate Block event in step 4) is
tracked as usage — typically milliseconds.

**TimeLimit expiry during continuous use (per-minute tick):**

1. Tick detects TimeLimit exceeded → adds B to `BlockedApps` → emits
   `BlockedAppsChanged`.
2. Plugin receives signal, sees the currently focused window is B, which is now
   in `BlockedApps` → **immediately** sends `Event(tag=2=Block, app="B")` →
   daemon writes Blocked(8).
3. Plugin shows overlay. No focus switch needed — the tick + immediate Block
   event catches continuous-use expiry.

**Subsequent focus attempts (user closes window and re-launches):**

1. Plugin sees B in `BlockedApps` → sends `Event(tag=2=Block, app="B")`.
2. Daemon writes Blocked(8) — terminates interval, no time accumulates.
3. `BlockedApps` unchanged → plugin keeps showing overlay.

**Block lifted (policy edited):**

1. Daemon removes B from `BlockedApps` → emits `BlockedAppsChanged`.
2. Plugin receives signal, sees the currently focused window is B, which is now
   removed from `BlockedApps` → **immediately** sends `Event(tag=0=Focus)` for B
   → daemon writes Focus(0), re-opens tracking.
3. Plugin removes overlay.

### Plugin Disconnect Handling

If the plugin's bus name disappears while a block is active, the overlay is gone
and enforcement is temporarily lost — the app runs without input trapping.

1. The app keeps running (the overlay was the only enforcement mechanism). The
   block remains in the daemon's `BlockedApps` — it is not cleared.
2. The dashboard is read-only regarding block state. `BlockedApps` persists.
3. When the plugin reconnects, it re-reads `BlockedApps` from the daemon and
   re-establishes overlays for all blocked apps. The daemon subscribes to
   `NameOwnerChanged` on the daemon's bus for `org.wellbeing.v1.Manager` and
   re-evaluates on reconnect — if the app is still focused and blocked,
   `BlockedApps` is already set, so the plugin shows the overlay.
4. Blocks are lifted only by policy edits, not by plugin disconnect/reconnect
   cycles.

### Startup Recovery — Plugin State Reconciliation

If the daemon crashes while an overlay is active, the plugin retains the overlay
(it keeps rendering on the compositor). On restart:

1. Daemon rebuilds `BlockedApps` by re-evaluating all focused apps (reads last
   events from DB + queries current focused app from plugin's `CurrentFocus`).
2. Daemon publishes `BlockedApps` — plugin reads it and reconciles its overlay
   set. Apps that were blocked before the crash are re-blocked.
3. If a blocked app is still focused, the plugin shows the overlay. If the app
   is no longer focused (user closed it during the crash), the block remains in
   `BlockedApps` — next focus triggers it.
