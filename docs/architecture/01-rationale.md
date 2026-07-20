# Design Rationale ("Why")

Three design decisions recur in review. Each is justified below; the
implementation details live in the topical docs linked from the
[overview](./README.md).

## Why Platform Abstraction?

The `Platform` trait isolates OS-specific code from the domain model and actors.
The OS-specific surface area is:

1. **Window events** — how to discover and receive focus/close events from the
   compositor.
2. **Block state exposure** — how the daemon publishes the set of currently
   blocked apps for the plugin to consume.
3. **App metadata** — how to resolve app_id to display name and icon.
4. **Power state** — how to detect suspend/shutdown for flush.

The domain model (tracking, policy, categorization, reports) is pure Rust
without OS dependencies. This means:

- The entire domain model and actor logic is testable with `MockPlatform`.
- Adding a new compositor (Hyprland → KWin → GNOME) requires only a new plugin
  implementing the same D-Bus interface, with zero changes to the daemon's
  `Platform` trait or actors.
- The `Platform` impl is selected at compile time via generics (`P: Platform`).
  No `dyn`, no vtable dispatch, no runtime overhead.

**What the platform does NOT abstract** (not needed):

- **Process control** — enforcement is overlay-only.
- **App metadata resolution** — metadata is resolved from the `app_categories`
  DB table, not from filesystem scanning or D-Bus portals.

See [02-platform.md](./02-platform.md) and
[03-linux-platform.md](./03-linux-platform.md).

## Why gpui?

- Retained-mode rendering (vs egui's immediate mode) — lower CPU usage.
- GPU-efficient (only redraws dirty regions).
- Proper text shaping with system font stack (Arabic, CJK, emoji).
- Native window management (multiple windows for overlay + dashboard).
- Pinned to a specific git commit for reproducibility (see
  [11-implementation-plan.md](./11-implementation-plan.md#open-questions) for
  the pin strategy).

## Why D-Bus for Plugin IPC (Not Per-Compositor Detection)

The compositor plugin is discovered dynamically on the system bus, with no
per-compositor feature gates or environment-variable detection:

1. **Zero detection** — the daemon publishes block state on its own D-Bus
   interface (`org.wellbeing.v1.Daemon.ActiveBlocks`). The plugin reads state
   from the daemon's well-known name. The daemon never probes
   compositor-specific env vars or socket paths.
2. **Single IPC contract** — all compositor plugins implement the same D-Bus
   interface. The daemon has one code path regardless of compositor.
3. **No feature gates** — the daemon doesn't need `#[cfg(feature = "hyprland")]`
   or similar. Plugin selection is a runtime D-Bus discovery concern.
4. **Plugin heterogeneity** — Hyprland runs a C++ `.so`, GNOME runs a JS
   extension, both speak the same D-Bus API. The daemon is indifferent.
5. **Graceful degradation** — if no plugin is registered on the bus at startup,
   the daemon logs a warning, shows a banner in the dashboard, and proceeds
   without block enforcement. If the plugin appears later (user loads it), the
   dashboard banner auto-dismisses. Block state accumulates regardless of
   plugin connectivity — overlays appear on reconnect.

**Mock testing** uses a `MockManagerClient` implementing the same interface — no
`MockCompositor` needed, no env var stubs, no feature flags.

The full contract is in [04-plugin-ipc.md](./04-plugin-ipc.md).
