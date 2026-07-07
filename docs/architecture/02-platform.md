# The Platform Trait

The central OS abstraction. Defined in `platform/mod.rs`. See the
[overview](./README.md) for where it fits in the two-binary split. The Linux
implementation lives in [03-linux-platform.md](./03-linux-platform.md); the
plugin D-Bus contract it talks to is in [04-plugin-ipc.md](./04-plugin-ipc.md).

## The Platform Trait

```rust
/// Platform-specific OS abstraction. The trait is a pure operation contract
/// — no constructor. Each impl provides its own builder/factory that guarantees
/// full initialization before any operation is accessible.
pub trait Platform: Send + Sync + 'static {
    type EventStream: Stream<Item = PlatformEvent> + Send + 'static;
    /// Show a blocking overlay on the target app window.
    /// Fire-and-forget — returns immediately after dispatching to plugin.
    /// The user's choice arrives via the EventStream as a UserAction variant.
    /// Returns an error if the compositor plugin is not available
    /// (not registered or disconnected from the bus).
    async fn show_overlay(&self, config: OverlayConfig) -> Result<()>;

    /// Hide an overlay immediately (non-blocking).
    /// Returns an error if the compositor plugin is not available.
    async fn hide_overlay(&self, app_id: &AppId) -> Result<()>;

    /// Send a desktop notification via org.freedesktop.Notifications.
    /// Used by Notify policies to alert the user when a time limit is exceeded.
    /// Non-blocking — fire-and-forget from the caller's perspective.
    async fn notify(&self, title: &str, body: &str) -> Result<()>;
}
```

### OverlayConfig

The `OverlayConfig` struct carries everything a compositor plugin needs to
render the block overlay. The same struct is used across all compositor backends
— serialized over D-Bus for the plugin, never containing compositor-specific
state:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct OverlayConfig {
    /// Which app to block — plugin resolves this to a window handle.
    pub app_id: AppId,
    /// The specific policy whose verdict produced this block. Carried into the
    /// `Overlay(show)` payload and echoed back (signed) in `UserAction`, so the
    /// daemon can re-derive the exact extend rule without keeping block state.
    pub policy_id: PolicyId,
    /// Why the block triggered.
    pub reason: BlockReason,
    /// Wall-clock time when the block started (also the token instance binding).
    pub blocked_since: SystemTime,
    /// Which action buttons to show: [Extra, Close] or [Close] only.
    pub available_actions: Vec<OverlayAction>,
}
```

No window geometry is passed — the plugin reads window dimensions directly from
compositor memory (frame-perfect overlay is the reason the plugin exists). See
the `org.wellbeing.v1.Manager` interface and Signed Overlay Tokens sections in
[04-plugin-ipc.md](./04-plugin-ipc.md) for the full overlay lifecycle and
zero-trust contract.

---

### Construction — Per-Platform Builders

The Platform trait does **not** define constructors. Each platform impl provides
its own builder or factory function with its required parameters encoded in
`new()`. This prevents the API error of calling operations on an uninitialized
platform — you cannot obtain a `&Platform` without baking all required
dependencies into the builder at compile time.

**LinuxPlatformBuilder** has no compositor-specific state — the daemon
communicates with whatever compositor plugin is registered on the system D-Bus
bus. No detection, no feature gates for compositor variants:

```rust
/// Linux platform builder. All required params in new(). build() returns
/// Result only for genuine IO failures (D-Bus connection).
pub struct LinuxPlatformBuilder;

impl LinuxPlatformBuilder {
    pub fn new() -> Self;

    /// Connect to system D-Bus, build platform with event stream.
    /// The plugin is discovered asynchronously via NameOwnerChanged.
    /// Platform::show_overlay() and hide_overlay() return errors until
    /// the plugin appears on the bus — after that, overlay calls
    /// reach the plugin.
    /// Event stream subscribes to FocusChanged signals from the plugin.
    pub async fn build(self) -> Result<(impl Platform, impl Stream<Item = PlatformEvent>)>;
}
```

**MockPlatform** has no builder — its constructor is infallible and takes
pre-seeded data directly. `show_overlay()` and `hide_overlay()` always succeed:

```rust
impl Platform for MockPlatform {
    type EventStream = MockEventStream;

    async fn show_overlay(&self, config: OverlayConfig) -> Result<()> {
        *self.last_overlay_config.lock().unwrap() = Some(config);
        Ok(())
    }

    async fn hide_overlay(&self, _app_id: &AppId) -> Result<()> { Ok(()) }
}

impl MockPlatform {
    /// Construct with pre-recorded event trace for tests.
    pub fn from_events(
        events: Vec<PlatformEvent>,
    ) -> (Self, impl Stream<Item = PlatformEvent>) {
        let events = VecDeque::from(events);
        let platform = Self {
            events: events.clone(),
            last_overlay_config: Arc::new(Mutex::new(None)),
        };
        let stream = MockEventStream { events };
        (platform, stream)
    }
}
```

**Interior mutability:** All `&self` Platform methods imply the impl uses
interior mutability (`Arc<Mutex<...>>`, `Atomic*`, or `RefCell`). This is by
design — the platform handle is shared across multiple actors. The impl manages
its own mutable state internally behind the `&self` API surface.

### Concurrency Model

The EnforcerActor is the sole caller of overlay operations (via
`Platform::show_overlay()` and `hide_overlay()`). All overlay operations are
dispatched to the compositor plugin over D-Bus, making each call an async IPC
round-trip. There is no shared mutable state between the daemon and the plugin —
all state is exchanged over the wire.

The daemon side uses `&self` on the Platform trait (not `&mut self`), but the
Linux impl's mutable state (D-Bus proxy) is behind interior mutability
(`Arc<tokio::sync::RwLock>`, `tokio::sync::Mutex`). The `Platform` impl is
concrete and known at compile time — actors are generic over `P: Platform`.

Overlay(v) is fire-and-forget. The UserAction signal is consumed by the
EnforcerActor's event loop like any other PlatformEvent. Multiple overlays can
be shown concurrently across different users.

### Event Model

```rust
pub enum PlatformEvent {
    /// A window is now focused. The plugin includes whether a block overlay
    /// is currently rendered on this window (used for crash recovery).
    WindowFocused {
        app_id: AppId,
        title: WindowTitle,
        pid: Pid,
        uid: Uid,
        overlay_shown: bool,
    },
    /// No window is focused (desktop, overview — NOT the lock screen; lock is
    /// its own `Locked` event emitted by the session watcher).
    Unfocused,
    /// User went idle (plugin idle signal). Pauses the open interval.
    Idle,
    /// User resumed activity. Unpauses the open interval.
    Resumed,
    /// User interacted with a block overlay. The Ed25519 signature has already
    /// been verified at the D-Bus boundary, so these fields are trusted:
    /// `app_id` + `action` are the plugin's window-domain assertion (the plugin
    /// is the window authority), and `policy_id` identifies the specific policy
    /// whose overlay was shown — multiple policies can block the same app with
    /// different extend rules. `blocked_since` and the signature are
    /// boundary-only (instance binding + tamper proofing) and are not carried
    /// into the domain event.
    UserAction {
        app_id: AppId,
        action: u32,
        policy_id: PolicyId,
    },
}

/// `Locked`, `LoggedOut`, `Slept`, and `ShutDown` are NOT `PlatformEvent`
/// variants. They are emitted directly into the event log by the session /
/// power watcher (`platform/linux/suspend.rs`) from systemd-logind signals —
/// bypassing the enforcer gate because they are terminal and need no policy
/// evaluation. They carry no `app_id` and simply close the open interval.
```

These events are the sole input to the system state machine. No platform
knowledge leaks beyond `PlatformEvent`. The tracker consumes window events for
session timing; the enforcer consumes them for policy evaluation.

**`overlay_shown` flag:** The compositor plugin includes this boolean in every
`WindowFocused` signal, telling the daemon whether a block overlay is already
rendered on that window. This is the primary recovery mechanism: after a
restart, the daemon need not query active overlays — the data arrives with the
next focus event.

**Synthetic events:** When the user grants extra time after a block, the
EnforcerActor inserts a synthetic `WindowFocused` event after writing the
extension. This opens a new focus interval, ensuring duration calculations
reflect actual post-grant usage. The synthetic event carries the last known PID
and window title from the pre-block session (see Event Processing Pipeline in
the [overview](./README.md#event-processing-pipeline)).
