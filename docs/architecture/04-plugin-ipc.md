# Plugin IPC (D-Bus)

The daemon and GUI communicate with the compositor plugin over the **daemon's
bus** — the **system bus** when the daemon runs in system mode (root), the
**session bus** when it runs in session mode (non-root). See
[13-deployment-modes.md](./13-deployment-modes.md) for bus/scope selection. Each
plugin instance claims a **unique** `org.wellbeing.v1.Manager.*` name and calls
`Daemon.RegisterPlugin` (reverse discovery — see
[06-daemon-dbus.md](./06-daemon-dbus.md#interface-definition)), so a single
daemon serves many plugin instances. The daemon authenticates each plugin by its
kernel-authenticated `SO_PEERCRED` uid (not by its claimed name).

**Plugin bus resolution uses the same 4-step algorithm as the GUI**
([13-deployment-modes.md](./13-deployment-modes.md#plugin-resolution)): the
plugin runs the identical `resolve_daemon_bus()` resolution (system present →
session present → activate system → activate session) and registers
`RegisterPlugin` on whichever daemon it finds. This guarantees exactly one
enforcing daemon per user — no double overlay.

No compositor detection, no socket path configuration, no feature gates.

## D-Bus Interface — `org.wellbeing.v1.Manager`

```xml
<node name="/org/wellbeing/Manager">
  <interface name="org.wellbeing.v1.Manager">

    <!-- Show or hide blocking overlay. The command variant is wrapped in a
         signed envelope the daemon signs with its Ed25519 private key; the
         plugin verifies it against the public key from
         org.wellbeing.v1.Daemon.DaemonPublicKey before acting. See
         [05-daemon-auth.md](./05-daemon-auth.md).
         command variant:
         ShowOverlayCmd { app_id: s, policy_id: t, reason: u, blocked_since: t, available_actions: au, signature: ay }
         HideOverlayCmd { app_id: s } -->
    <method name="Overlay">
      <arg name="envelope" type="v" direction="in"/>   <!-- SignedEnvelope -->
      <arg name="ack" type="b" direction="out"/>
    </method>

    <!-- User interacted with a block overlay (action button pressed).
         The plugin is the authority on window/click facts, so app_id + action
         are its assertion. policy_id + blocked_since + signature are the
         daemon-issued, Ed25519-signed token echoed back by the plugin (see
         "Signed Overlay Tokens" below) — the daemon verifies the signature
         before trusting policy_id. -->
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

    <!-- User idle state changed. `idle=true` → emit `Idle` PlatformEvent
         (pause the open interval); `idle=false` → emit `Resumed` (unpause).
         The plugin tracks keyboard/mouse/touchpad/video-player activity. -->
    <signal name="ActivityChanged">
      <arg name="idle" type="b"/>
    </signal>

    <!-- Readable property: current session state.
         Returns the SAME FocusVariant as the FocusChanged signal:
           1 = Desktop, 2 = App {app_id, title, pid, uid, overlay_shown}
         The signal is fire-and-forget and does not persist its value, so this
         property is the canonical, queryable source of truth (GUI reads it on
         startup; daemon uses it for crash-recovery reconciliation). -->
    <property name="CurrentSession" type="v" access="read"/>

  </interface>
</node>
```

**Why `CurrentSession`?** D-Bus signals are fire-and-forget — they do not
persist their last value, so a GUI that subscribes after the fact misses the
current state. `CurrentSession` is a readable D-Bus property that returns the
**same value as the `FocusChanged` signal** (same `FocusVariant` encoding),
giving clients a queryable, always-current source of truth on startup and for
reconciliation. The signal remains useful as a lightweight change notification.
The daemon also uses `CurrentSession` on restart to reconcile each plugin
instance's overlays with plugin state (see
[Multi-Instance Plugin Support](#multi-instance-plugin-support)).

## Per-App Multi-Overlay Model

Blocking enforcement is keyed by `app_id`, **never by window**. The daemon is
**window-count agnostic**: it addresses only an `app_id`. Whether the app has
one window or fifty, the daemon sends exactly one `Overlay(show)` and one
`Overlay(hide)` per `app_id` — it never enumerates, counts, or tracks individual
windows.

The plugin treats **every window of the `app_id` as a single logical surface**.
One `show` command blocks _all_ of that app's windows at once; one `hide`
command unblocks _all_ of them. There is no per-window blocking state and no
partial block — a blocked `app_id` is blocked in full.

When the daemon decides to block an app, it sends an `Overlay(show)` containing
that app's `app_id`. The plugin renders a block overlay over every window owned
by the app and traps **both mouse and keyboard input** on each blocked window,
so the app cannot be interacted with until the user presses an overlay button or
the block is lifted. The overlay presents the daemon-issued action buttons
(`available_actions`); a click on a button (or the swallowed keyboard) is
reported back to the daemon via the `UserAction` signal carrying that app's
signed token.

Multiple distinct apps can be blocked at the same time. The plugin tracks an
unordered set of active overlays keyed by `app_id`. Each set entry holds:

- The daemon-issued signed token (`policy_id`, `blocked_since`, `signature`)
- The set of window handles for that app (populated from compositor memory;
  TODO: window-handle set tracking is currently stubbed)

This replaces the previous single-active-overlay behavior where a second blocked
app would clobber the first. The daemon remains the source of truth for blocking
decisions — it sends show/hide commands per `app_id`, and the plugin maintains
the overlay state.

**Overlay identity:** The `app_id` field inside the `Overlay(v)` variant is the
overlay key. A hide command targets a specific `app_id`; a show command for an
already-blocked `app_id` updates that app's overlay token (e.g. on daemon
restart recovery). The plugin validates `app_id` at the D-Bus boundary via the
`AppId` newtype and rejects empty/invalid values (zero-trust).

## Enums (all serialized as integer discriminants in variants)

```text
Overlay variant command:
  show:  {app_id:s, policy_id:t, reason:u, blocked_since:t, available_actions:au, signature:ay}
         (app_id is the overlay identity/key — identifies which app's overlay
          to show. signature = Ed25519 over app_id ‖ policy_id ‖ blocked_since ‖
          instance_id, the daemon-issued token the plugin echoes back in
          UserAction; the OUTER request itself is wrapped in a SignedEnvelope —
          see 05-daemon-auth.md)
  hide:  {app_id:s}
         (app_id identifies which app's overlay to remove)

FocusChanged:     variant payload = Option<WindowInfo>

WindowInfo {
    app_id: s,
    title: s,
    pid: u,
    uid: u,
    overlay_shown: b,
}

// CurrentSession returns the FocusChanged FocusVariant (see above):
//   1 = Desktop
//   2 = App { app_id: s, title: s, pid: u, uid: u, overlay_shown: b }
// The property and the signal share one encoding so their values are
// identically comparable — there is no separate "SessionState" tag.

UserAction signal payload:
  app_id: s, action: u, policy_id: t, blocked_since: t, signature: ay
  (the plugin is the authority on app_id + action; policy_id + blocked_since +
   signature are the daemon-issued, Ed25519-signed token echoed back — the
   plugin looks up the token for the clicked app's `app_id` from its active
   overlays set before emitting. See [05-daemon-auth.md](./05-daemon-auth.md))

PluginInstanceId:  opaque id (e.g. "<uid>@<session>") uniquely identifying one
  plugin process. Multiple plugin instances (multiple Hyprland/users) each
  expose org.wellbeing.v1.Manager at a UNIQUE well-known name and are tracked
  separately by the daemon (see Multi-Instance Plugin Support).

OverlayAction:     0=Extra  1=Close
BlockReason:       0=AppTimeLimit  1=CategoryTimeLimit  2=AppBlock  3=CategoryBlock
```

`Overlay(v)` is fire-and-forget — the daemon sends a show/hide command variant,
the plugin renders or removes the overlay, and the method returns immediately.
User actions (button clicks) arrive asynchronously via the `UserAction` signal.

`FocusChanged` signal carries the current focused window state as an
`Option<WindowInfo>` variant. The daemon subscribes to this signal and maps
directly to `PlatformEvent`. The GUI also subscribes for the active window
display.

## Idle Detection

`Idle`/`Resumed` are produced by the compositor plugin, not logind. The plugin
already tracks user activity (keyboard, mouse, touchpad, and video-player
playback) and exposes it via the new `ActivityChanged` D-Bus signal on
`org.wellbeing.v1.Manager`. The daemon's `ManagerClient` subscribes and maps
`idle=true` → `Idle` (pause), `idle=false` → `Resumed` (unpause) PlatformEvents,
which flow through the same `PlatformEvent` stream as `FocusChanged`.

Key points:

- `Idle`/`Resumed` carry **no** `app_id`; the app they pause is the open
  interval from the most recent `WindowFocused`.
- `Idle` is the ONLY event that pauses an interval. Suspend/lock/logout/shutdown
  CLOSE it instead (see
  [03-linux-platform.md](./03-linux-platform.md#power--session-state-handling)).
- The plugin is responsible for idle debounce (e.g. a min-dwell before emitting
  `Idle`) so brief input gaps don't create noise segments.

## Rust Side (daemon, zbus)

The daemon's platform module connects to the **daemon's own bus** (system in
system mode, session in session mode — resolved at startup per
[13-deployment-modes.md](./13-deployment-modes.md)) and discovers the plugin:

```rust
use zbus::proxy;
use zvariant::Type;

#[derive(Type, Serialize, Deserialize)]
#[zvariant(signature = "v")]
pub struct WindowInfo {
    pub app_id: String,
    pub title: String,
    pub pid: u32,
    pub uid: u32,
    pub overlay_shown: bool,
}

/// Raw `UserAction` signal payload. `signature` is the Ed25519 token the daemon
/// issued when it showed the overlay; it is verified (see "Signed Overlay
/// Tokens") BEFORE this becomes the domain `PlatformEvent::UserAction`. A
/// `Signature` newtype wraps the raw bytes so an unverified token can never
/// enter domain logic.
#[derive(Type, Serialize, Deserialize, Debug)]
pub struct UserActionEvent {
    pub app_id: String,
    pub action: u32,
    pub policy_id: u64,
    pub blocked_since: u64,
    pub signature: Vec<u8>,
}

#[proxy(
    interface = "org.wellbeing.v1.Manager",
    default_service = "org.wellbeing.v1.Manager",
    default_path = "/org/wellbeing/Manager"
)]
trait Manager {
    /// Single Overlay(v) method — command variant encodes show/hide.
    /// The show variant also carries `policy_id` + `signature` so the plugin
    /// can echo them back in the `UserAction` signal.
    /// `app_id` inside the variant identifies which app's overlay to
    /// show/hide; the overlay is keyed by `app_id` on the plugin side.
    async fn overlay(&self, command: &zvariant::OwnedValue) -> zbus::Result<bool>;

    /// CurrentSession property — returns the SAME FocusVariant as the
    /// FocusChanged signal. It is the queryable source of truth for current
    /// session state (the signal is ephemeral).
    #[zbus(property)]
    async fn current_session(&self) -> zbus::Result<zvariant::OwnedValue>;

    /// User clicked an overlay button. `policy_id` + `blocked_since` +
    /// `signature` are the daemon-issued, Ed25519-signed token echoed by the
    /// plugin (the plugin looks up the token for the clicked `app_id` from
    /// its active overlays set); `action` is the plugin's window-domain
    /// assertion. The daemon verifies `signature` before trusting `policy_id`.
    #[zbus(signal)]
    fn user_action(
        &self,
        app_id: &str,
        action: u32,
        policy_id: u64,
        blocked_since: u64,
        signature: &[u8],
    ) -> zbus::Result<()>;
}

/// One compositor plugin instance. Created when a plugin registers (or is
/// discovered) and dropped when its bus name disappears. Holds the D-Bus proxy
/// plus the instance identity used for routing and token binding.
pub struct ManagerClient {
    instance_id: PluginInstanceId,
    uid: Uid,
    proxy: ManagerProxy<'static>,
}

/// Tracks every connected plugin instance. The daemon accepts ANY plugin that
/// calls `Daemon.RegisterPlugin` — it does not pin a single well-known name.
/// Each instance is keyed by its unique `PluginInstanceId` and
/// kernel-authenticated `uid`. Overlay calls are routed to the instance whose
/// `uid` matches the blocked app's owner.
pub struct PluginRegistry {
    clients: HashMap<PluginInstanceId, ManagerClient>,
    /// Ed25519 keypair for signing. Generated fresh in memory each daemon
    /// start, never persisted. Signs BOTH the request envelope (so the plugin
    /// can verify the Overlay call came from the daemon) and the echo-back
    /// token (app_id ‖ policy_id ‖ blocked_since ‖ instance_id) the plugin
    /// carries back in `UserAction`. One key, two signatures.
    keypair: ed25519_dalek::SigningKey,
}

impl PluginRegistry {
    /// Route an overlay to the plugin instance owned by `uid`.
    /// Returns `PluginNotConnected` if no instance for that user is registered.
    pub async fn show_overlay_for(&self, config: &OverlayConfig, uid: Uid) -> Result<()> {
        let client = self.clients.values()
            .find(|c| c.uid == uid)
            .ok_or_else(|| anyhow!("no plugin for uid {}", uid))?;
        client.show_overlay(config, &self.keypair).await
    }

    /// Verify an incoming `UserAction` token: Ed25519-verify the echoed
    /// signature over (app_id ‖ policy_id ‖ blocked_since ‖ instance_id) with
    /// the daemon's own public key. On success `policy_id` is trusted as
    /// daemon-issued; the daemon then looks up the policy from its own DB (it
    /// never trusts policy *values* from the signal). Returns the verified
    /// tuple, or `None` to drop the event on mismatch / unknown instance.
    fn verify_user_action(
        &self,
        ev: &UserActionEvent,
        instance_id: &PluginInstanceId,
    ) -> Option<(AppId, u32, PolicyId)> {
        let msg = [
            ev.app_id.as_bytes(),
            &ev.policy_id.to_be_bytes(),
            &ev.blocked_since.to_be_bytes(),
            instance_id.as_bytes(),
        ].concat();
        let sig = ed25519_dalek::Signature::from_slice(&ev.signature).ok()?;
        if self.keypair.verifying_key().verify(&msg, &sig).is_err() {
            return None;
        }
        Some((AppId::new(ev.app_id.clone())?, ev.action, PolicyId::new(ev.policy_id)))
    }
}

/// Signals are subscribed when each instance registers (before any overlay is
/// shown for it), so there is no runtime ordering hazard: a `UserAction` can
/// never arrive at a connection with no handler. The registry verifies each
/// token and forwards only verified events into the EnforcerActor loop.
/// Request envelope the daemon signs and the plugin verifies. Serialized as
/// the `v` argument of `Overlay`. See [05-daemon-auth.md](./05-daemon-auth.md).
#[derive(Type, Serialize, Deserialize)]
pub struct SignedEnvelope {
    pub payload: zvariant::OwnedValue, // show/hide command variant, verbatim
    pub issued_at: u64,                // unix ms; plugin rejects if outside ±SKEW
    pub signature: Vec<u8>,            // Ed25519(payload ‖ issued_at)
}

impl ManagerClient {
    /// Show overlay — fire-and-forget. Signs the UserAction echo-back token at
    /// show time (embedded in the payload) AND wraps the whole call in a
    /// `SignedEnvelope` (verified by the plugin). Same `keypair` for both.
    async fn show_overlay(&self, config: &OverlayConfig, keypair: &ed25519_dalek::SigningKey) -> Result<()> {
        let blocked_since_ms = config.blocked_since
            .duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64;
        // Inner token for the UserAction echo-back (verified by the daemon later).
        let token = keypair.sign(&[
            config.app_id.as_ref().as_bytes(),
            &config.policy_id.as_u64().to_be_bytes(),
            &blocked_since_ms.to_be_bytes(),
            self.instance_id.as_bytes(),
        ].concat());
        let cfg = OverlayConfigPayload {
            app_id: config.app_id.as_ref().to_owned(),
            policy_id: config.policy_id.as_u64(),
            reason: config.reason as u32,
            blocked_since: blocked_since_ms,
            available_actions: config.available_actions
                .iter().map(|a| *a as u32).collect(),
            signature: token.to_bytes().to_vec(),
        };
        let payload = zvariant::Value::Dict([("show".to_owned(), zvariant::Value::from(cfg))]);
        self.proxy.overlay(&self.sign_envelope(payload, keypair).await?).await?;
        Ok(())
    }

    /// Hide an overlay immediately (non-blocking). Same signed envelope.
    async fn hide_overlay(&self, app_id: &AppId, keypair: &ed25519_dalek::SigningKey) -> Result<()> {
        let payload = zvariant::Value::Dict([("hide".to_owned(), zvariant::Value::new(app_id.as_ref()))]);
        self.proxy.overlay(&self.sign_envelope(payload, keypair).await?).await?;
        Ok(())
    }

    /// Wrap a show/hide command variant in a `SignedEnvelope`. Signs
    /// `payload ‖ issued_at` with the daemon's private key; the plugin verifies
    /// it against `DaemonPublicKey` before acting.
    async fn sign_envelope(
        &self,
        payload: zvariant::Value<'_>,
        keypair: &ed25519_dalek::SigningKey,
    ) -> Result<zvariant::OwnedValue> {
        let issued_at = SystemTime::now()
            .duration_since(UNIX_EPOCH).unwrap().as_millis() as u64;
        let payload_bytes = zvariant::to_bytes(&payload)?;
        let sig = keypair.sign(&[payload_bytes.as_slice(), &issued_at.to_be_bytes()].concat());
        let envelope = SignedEnvelope {
            payload: payload.try_into()?,
            issued_at,
            signature: sig.to_bytes().to_vec(),
        };
        Ok(zvariant::Value::from(envelope).try_into()?)
    }
}
```

## C++ Plugin Side (sdbus-cpp v2)

```cpp
#include <sdbus-c++/sdbus-c++.h>

// ── Variant tags (D-Bus variant discriminants) ──────────────────────────
// FocusVariantTag — discriminator for FocusChanged / CurrentSession variant
enum class FocusVariantTag : uint32_t {
    Desktop = 1,  // No window focused (wallpaper / empty)
    App     = 2,  // An app window is focused
};

// CurrentSession reuses FocusVariantTag (defined above) so the property and the
// FocusChanged signal encode state identically. The earlier SessionStateTag
// discriminator is retired — there is no longer a separate "session" tag.

// ── Helpers for variant encoding ────────────────────────────────────────

/// Encode an Option<WindowInfo> as a D-Bus variant for FocusChanged.
///
/// Encoding:
///   None          → variant(uint32 FocusVariantTag::Desktop)
///   Some{...}     → variant(struct{FocusVariantTag::App, app_id, title, pid,
///                                   uid, overlay_shown})
auto windowInfoToVariant(const std::optional<WindowInfo>& info) -> sdbus::Variant {
    if (!info.has_value()) {
        return sdbus::Variant{static_cast<uint32_t>(FocusVariantTag::Desktop)};
    }
    return sdbus::Variant{std::tuple{
        static_cast<uint32_t>(FocusVariantTag::App),
        info->appId, info->title, info->pid, info->uid, info->overlayShown,
    }};
}

/// Build the CurrentSession variant.
///
/// Returns the SAME FocusVariant as the FocusChanged signal so a late-joining
/// client can read identical state from the readable property (the signal is
/// ephemeral). Both call windowInfoToVariant(currentFocus).
auto buildSessionVariant() -> sdbus::Variant {
    return windowInfoToVariant(g_ctx->currentFocus);
}

// ── Ed25519 verification helpers (stub — FAILS CLOSED) ───────────────────

auto fetchDaemonPublicKey(sdbus::IConnection& conn)
    -> std::pair<std::string, std::vector<uint8_t>>;   // see 05-daemon-auth.md
bool verifyEnvelope(sdbus::IConnection& conn,
                  const sdbus::Variant& payload, uint64_t issuedAt,
                  const std::vector<uint8_t>& sig);     // see 05-daemon-auth.md

// ── WellbeingManager — D-Bus org.wellbeing.v1.Manager interface ───────────

class WellbeingManager {
    std::shared_ptr<sdbus::IConnection> m_conn;
    std::unique_ptr<sdbus::IObject> m_object;
    std::shared_ptr<LockManager> m_lockManager;   // compositor-side overlay state

public:
    WellbeingManager(std::shared_ptr<LockManager> lockManager,
                     std::shared_ptr<sdbus::IConnection> connection)
        : m_conn(std::move(connection))
        , m_object(sdbus::createObject(*m_conn,
                     sdbus::ObjectPath{"/org/wellbeing/Manager"}))
        , m_lockManager(std::move(lockManager))
    {
        // sdbus-c++ v2: one addVTable per interface
        m_object
            ->addVTable(
                sdbus::registerMethod("Overlay").implementedAs(
                    [this](sdbus::Variant envelope) -> bool {
                        return handleOverlay(envelope);
                    }),
                sdbus::registerProperty("CurrentSession").withGetter(
                    []() -> sdbus::Variant {
                        return buildSessionVariant();  // reads g_ctx->currentFocus
                    }),
                sdbus::registerSignal("UserAction")
                    .withParameters<std::string, uint32_t,
                                    uint64_t, uint64_t,
                                    std::vector<uint8_t>>(
                        {"app_id", "action", "policy_id",
                         "blocked_since", "signature"}),
                sdbus::registerSignal("FocusChanged")
                    .withParameters<sdbus::Variant>({"window"}),
                sdbus::registerSignal("ActivityChanged")
                    .withParameters<bool>({"idle"}))
            .forInterface("org.wellbeing.v1.Manager");

        // Wire LockManager button clicks → our emitUserAction.
        // AppId → raw string conversion happens at this D-Bus boundary.
        m_lockManager->setUserActionCallback(
            [this](const AppId &appId, uint32_t action) {
                emitUserAction(appId.value(), action);
            });

        registerWithDaemon();
    }

    // ── Reverse discovery ──────────────────────────────────────────────
    // m_conn is the named connection against the bus type returned by
    // resolveDaemonBus() (see 13-deployment-modes.md — C++ Side). The plugin
    // calls RegisterPlugin on whichever daemon owns org.wellbeing.v1.Daemon
    // on that bus.
    void registerWithDaemon() {
        auto daemon = sdbus::createProxy(
            *m_conn, sdbus::ServiceName{"org.wellbeing.v1.Daemon"},
            sdbus::ObjectPath{"/org/wellbeing/Daemon"});
        daemon->callMethod("RegisterPlugin")
            .onInterface("org.wellbeing.v1.Daemon")
            .withArguments(instanceId());
    }

    // ── Signals ─────────────────────────────────────────────────────────
    void emitUserAction(const std::string& appId, uint32_t action) {
        // Look up the per-app token from LockManager's active overlays.
        const auto id = AppId::from_unchecked(appId);
        m_object->emitSignal("UserAction")
            .onInterface("org.wellbeing.v1.Manager")
            .withArguments(appId, action,
                           m_lockManager->activePolicyId(id),
                           m_lockManager->blockedSince(id),
                           m_lockManager->activeSignature(id));
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

    // ── Instance identity ──────────────────────────────────────────────
    static std::string instanceId() { /* "<uid>@<session>" */ return "..."; }
    static std::string wellKnownBusName() {
        return "org.wellbeing.v1.Manager." + instanceId();
    }

private:
    // ── Overlay dispatch — envelope parsing + show/hide ────────────────
    // See 05-daemon-auth.md for the full verification chain (Ed25519 +
    // freshness window). This snippet shows the dispatch structure.
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
        } catch (const sdbus::Error&) {
            return false; // malformed envelope
        }

        // Verify Ed25519 signature + freshness window (see 05-daemon-auth.md)
        if (!verifyEnvelope(*m_conn, payload, issuedAt, sig))
            return false;

        // Dispatch on inner payload variant via try/catch probe
        return tryShowOverlay(payload) || tryHideOverlay(payload);
    }

    /// Attempt to parse and apply a "show" overlay variant.
    /// Returns true on success, false if payload is not a show variant.
    auto tryShowOverlay(sdbus::Variant& payload) -> bool {
        try {
            auto show = payload.get<std::tuple<
                std::string, uint64_t, uint32_t, uint64_t,
                std::vector<uint32_t>, std::vector<uint8_t>>>();
            // Zero-trust boundary gate: validate appId before entering
            // LockManager (which enforces the AppId non-empty invariant).
            auto appId = AppId::from_raw(std::get<0>(show));
            if (!appId) return false;
            auto policyId    = std::get<1>(show);
            auto reason      = std::get<2>(show);
            auto blockedSince = std::get<3>(show);
            auto actions     = std::get<4>(show);
            auto innerSig    = std::get<5>(show);

            m_lockManager->showOverlay(*appId, policyId, reason,
                                       blockedSince, actions, innerSig);
            return true;
        } catch (const sdbus::Error&) {
            return false;
        }
    }

    /// Attempt to parse and apply a "hide" overlay variant.
    auto tryHideOverlay(sdbus::Variant& payload) -> bool {
        try {
            auto rawAppId = payload.get<std::string>();
            auto appId = AppId::from_raw(rawAppId);
            if (!appId) return false;
            m_lockManager->hideOverlay(*appId);
            return true;
        } catch (const sdbus::Error&) {
            return false;
        }
    }
};
```

## Multi-instance plugin support

The daemon does **not** pin a single `org.wellbeing.v1.Manager` name. Any plugin
that wants to talk to the daemon calls `Daemon.RegisterPlugin(instance_id)` on
startup. Because a D-Bus well-known name is unique per connection, each plugin
instance claims a **unique** name (e.g.
`org.wellbeing.v1.Manager.<uid>.<sess>`); the daemon learns the caller's real
identity from `SO_PEERCRED` (kernel-authenticated uid) and its unique bus name,
and stores a `ManagerClient` per instance in `PluginRegistry`. Signal
subscriptions are established at registration time — _before_ any overlay is
shown to that instance — so there is no runtime ordering hazard (a `UserAction`
can never arrive at a connection with no handler). `NameOwnerChanged` for
`org.wellbeing.v1.Manager.*` drops the instance's client on disconnect.

```
Daemon starts
  │
  ├── zbus::Connection::system()
  ├── Expose Daemon.RegisterPlugin(instance_id)
  └── Watch NameOwnerChanged for org.wellbeing.v1.Manager.*

Plugin appears (calls RegisterPlugin):
  ├── Daemon reads caller's SO_PEERCRED uid + unique bus name
  ├── Creates ManagerClient for that instance
  ├── Subscribes to FocusChanged + (verified) UserAction streams
  ├── Calls CurrentSession → reconcile this instance's overlays with DB
  └── Routes future overlays for uid → this instance

Plugin disconnects (NameOwnerChanged):
  ├── Drop that instance's ManagerClient
  └── Blocks for its user lift (no overlay possible) until it reconnects
```

**Why uid authentication (not name authentication):** The plugin's D-Bus policy
(`context="default"` for `own`) lets _any_ process on the system bus claim a
`org.wellbeing.v1.Manager.*` name, and v1 accepts any uid (open multi-user
model). We therefore do **not** trust the claimed name — we trust the
kernel-authenticated `SO_PEERCRED` uid. A forged `UserAction` from an impostor
is rejected by the Ed25519 signature check (the impostor lacks the daemon's
private key), not by name matching. Each instance's actions are scoped to its
uid's policies/usage.

If a plugin appears after the daemon started, registration wires it up and
overlay calls for its user start succeeding. If it disappears mid-session, its
client is dropped, overlay calls for its user fail, and the block lifts until
reconnect (see
[Plugin Disconnect Handling](#signed-overlay-tokens-zero-trust-useraction)).

## Signed Overlay Tokens (Zero-Trust `UserAction`)

The plugin is the **window authority**: a `UserAction` from the verified plugin
proves the overlay was shown and the user clicked a button. The daemon does
**not** re-check "is the window actually blocked?" via `overlay_shown`/DB — it
trusts the window-domain fact (exactly as it already trusts `FocusChanged` /
`CurrentSession` from the same plugin).

But `policy_id` is **daemon-owned policy data**. To let the plugin carry it back
without the daemon trusting it blindly, the daemon signs a token when it shows
the overlay:

- **Signed at show time:**
  `Ed25519(priv_key, app_id ‖ policy_id ‖ blocked_since ‖ plugin_instance)`,
  where `priv_key` is the daemon's Ed25519 private key — generated **in memory
  at daemon start**, never persisted, and reused to sign the request envelope
  (see below). The token rides inside the `Overlay(show)` payload.
- **Echoed by plugin:** the plugin stores the token and emits it back verbatim
  in `UserAction` alongside the user's `action` (its window-domain assertion).
- **Verified on receipt:** the daemon verifies the Ed25519 signature over the
  received `(app_id, policy_id, blocked_since, plugin_instance)` with its _own_
  public key. Mismatch → drop (rejects impersonation/tampering). Match →
  `policy_id` is trusted as daemon-issued; the daemon then looks up
  `policy_config` (time*limit / extra_seconds) from its **own DB** by
  `policy_id`. The signature authenticates the \_identifier* + instance binding;
  it never authenticates policy _values_ — those are always re-derived.

### Request signing (daemon → plugin) — NEW

The token above authenticates the `policy_id` the _plugin carries back_ to the
daemon. It does **not** authenticate the `Overlay` _request itself_. To stop a
local impostor from calling `Overlay` on the plugin, the daemon also signs the
request. The `Overlay` method now takes a `SignedEnvelope` (see
[05-daemon-auth.md](./05-daemon-auth.md) for the full algorithm):

- **Wrapped at send time:** `Ed25519(priv_key, payload ‖ issued_at)`, where
  `payload` is the show/hide command variant verbatim and `issued_at` is unix
  ms. Same keypair as the echo-back token — one key, two signatures.
- **Verified by plugin:** the plugin reads `DaemonPublicKey` (on demand) and
  verifies the signature, and checks `issued_at` is within ±SKEW (stateless, no
  nonce cache). Invalid or stale → the plugin **drops the call** and shows
  nothing. Only the holder of the daemon's private key can drive overlays.

**Key properties:**

- **`action` is not signed.** It is supplied by the plugin at click time and is
  the plugin's attestation of the click (window authority). The signature proves
  the token is genuine; it does not cover the action. This is correct because
  the daemon cannot know the action at show time, and the plugin holds no
  signing key.
- **No daemon-side `active_blocks` map.** The signal carries everything the
  daemon needs (`app_id` + authenticated `policy_id`); the `EnforcerActor`
  re-derives the policy from `policy_id`. Re-derivation is deterministic because
  a blocked app's `daily_usage` is frozen while the overlay traps input, so the
  same policy/extend-rule is resolved. Multiple policies can block the same app
  with different extend rules, so the _exact_ `policy_id` travels with the
  overlay and back rather than being re-guessed.
- **Instance binding.** Including `plugin_instance` in the signature means a
  token issued to plugin A cannot be replayed against plugin B; the daemon
  confirms the `policy_id`'s owner matches the instance's uid (defense-in-depth
  on the per-user boundary). The request envelope proves _daemon origin_ (only
  the daemon holds the private key), independent of instance.
- **Replay is naturally bounded.** The overlay is dismissed after the first
  action, so a replayed token with the same `blocked_since` finds no active
  block instance for that window → dropped. No nonce cache is needed (which
  would reintroduce active tracking). Pre-restart tokens are invalid after a
  restart (key regenerated), so a click during a daemon-restart gap is
  harmlessly dropped; the overlay is re-shown with a fresh token and the user
  clicks again.

## Delegation Chain

```
platform.show_overlay(config)
  → LinuxPlatform::show_overlay(config)
    → self.manager.show_overlay(config)
      → D-Bus Overlay(show_command)
      → returns immediately (ack)

platform.hide_overlay(app_id)
  → LinuxPlatform::hide_overlay(app_id)
    → self.manager.hide_overlay(app_id)
      → D-Bus Overlay(hide_command)
      → returns immediately (ack)

UserAction signal arrives asynchronously from plugin
  → EventStream yields UserAction { app_id, action }
  → EnforcerActor processes like any other PlatformEvent
```

The compositor plugin is the **sole** overlay renderer. On Linux, the Platform
impl delegates to the D-Bus `ManagerClient`, which calls `Overlay(v)` on the
plugin and returns immediately after dispatch. User actions (button clicks)
arrive asynchronously via the `UserAction` signal, which the LinuxPlatform
forwards into the PlatformEvent stream. If the plugin is not connected,
`show_overlay` / `hide_overlay` return an error, which actors handle as a
degraded experience (dashboard shows warning banner, block enforcement pauses).
