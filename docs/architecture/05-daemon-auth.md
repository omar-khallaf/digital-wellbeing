# Daemon Authentication & Plugin Request Signing

The daemon authenticates the D-Bus method calls it makes to the compositor
plugin so the plugin can prove a request came from the running daemon and reject
forged `Overlay` calls from any other local process.

> Canonical spec. The D-Bus interface definitions live in
> [06-daemon-dbus.md](./06-daemon-dbus.md) (`org.wellbeing.v1.Daemon`) and
> [04-plugin-ipc.md](./04-plugin-ipc.md) (`org.wellbeing.v1.Manager`); this
> document owns the signing design, key management, and verification algorithms.

## Overview

### Threat

The plugin's `Overlay` method is exposed on the system bus under a
`org.wellbeing.v1.Manager.*` name that any local process may claim (the bus
`own` policy is open — see
[04-plugin-ipc.md](./04-plugin-ipc.md#multi-instance-plugin-support)). Today the
plugin trusts **any** caller of `Overlay`. A local process can therefore:

- spoof a block overlay (`Overlay(show)`) for an arbitrary app, or
- cancel a real block (`Overlay(hide)`) by hiding the daemon's overlay.

The plugin's own signals (`FocusChanged`, `UserAction`) are _plugin→daemon_ and
out of scope here; the daemon already authenticates the plugin by its kernel
`SO_PEERCRED` uid.

### Goal

The daemon holds an **Ed25519 private key** and publishes the corresponding
**public key** on its D-Bus interface. Every `Overlay` request the daemon sends
is signed; the plugin reads the public key and verifies the signature before
acting. Only the holder of the private key (the running daemon) can drive
overlays.

## Design Decisions

| #   | Decision                                                                                             | Rationale                                                                                                                                                     |
| --- | ---------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1   | Public key is a D-Bus **property** (`DaemonPublicKey`), not a signal                                 | The plugin reads it **on demand** (each time it needs to verify), so it always holds the current key. A signal would push a key the plugin might cache stale. |
| 2   | Keypair is **in memory**, generated at every daemon start, **never persisted**                       | Avoids a private-key-at-rest secret.                                                                                                                          |
| 3   | **One** Ed25519 keypair serves both directions: request signing _and_ `policy_id` token verification | A single key drives both signatures — no separate symmetric key to manage.                                                                                    |
| 4   | **Stateless** replay protection (timestamp window, no nonce cache)                                   | `Overlay` is idempotent; a within-window replay is harmless. Stateless keeps the plugin simple and avoids active tracking.                                    |

## Key Management

- An **Ed25519** keypair (`SigningKey` / `VerifyingKey`) is generated in memory
  at daemon startup (e.g. via `ed25519-dalek` + `rand`). It is **never written
  to disk** and is dropped on shutdown.
- The **public key** (32 bytes) plus a `key_id` is exposed through the
  `DaemonPublicKey` property on `org.wellbeing.v1.Daemon`.
- `key_id` is an opaque id (e.g. a UUID) that **changes every start** because
  the key is regenerated. It lets the plugin detect a key change; because the
  plugin reads the property on demand, a stale key is never used.
- The private key is held only in the daemon's `PluginRegistry` / identity
  module. It is zeroized on drop (`zeroize`).

### Dependencies (daemon)

- `ed25519-dalek` — Ed25519 sign/verify.
- `rand` — keypair generation.
- `zeroize` — private-key wipe.
- No new dependency in `wellbeing-core`; signing/verification live at the
  `platform/linux` infrastructure boundary (`ManagerClient`), where byte-level
  crypto is permitted. Newtypes (`DaemonPublicKey`, `Signature`, `KeyId`) wrap
  raw bytes so they never enter domain structs (per AGENTS.md type-purity rule).

### Dependencies (plugin, C++)

- **libsodium** (`crypto_sign_verify_detached`) for Ed25519 verification — the
  daemon-issued `policy_id` token is _not_ verified by the plugin, so the plugin
  only needs the verify path. Alternatives: OpenSSL EVP / standalone `ed25519` C
  lib.

## D-Bus Surface

### `org.wellbeing.v1.Daemon` — new property

```xml
<!-- Ed25519 public key, regenerated in memory each daemon start.
     Open to all (it is a public key) — no RBAC, no SO_PEERCRED gate.
     Plugin reads this property on demand to verify Overlay requests.
     key_id changes every start so a stale key is detectable. -->
<property name="DaemonPublicKey" type="(sy)" access="read">
  <!-- (key_id: s, public_key: ay) -->
</property>
```

(Full interface in [06-daemon-dbus.md](./06-daemon-dbus.md).)

### `org.wellbeing.v1.Manager` — `Overlay` takes a signed envelope

```xml
<!-- Overlay command, wrapped in a signed envelope.
     The plugin reads org.wellbeing.v1.Daemon.DaemonPublicKey and verifies
     `signature` over (payload ‖ issued_at) before acting. Unsigned /
     unverifiable calls are dropped. -->
<method name="Overlay">
  <arg name="envelope" type="v" direction="in"/>   <!-- SignedEnvelope -->
  <arg name="ack" type="b" direction="out"/>
</method>
```

`SignedEnvelope` is a struct serialized as the `v` argument:

```text
SignedEnvelope {
    payload:    v,    # the show/hide command variant, verbatim
    issued_at:  t,    # unix ms; plugin rejects if outside ±SKEW (stateless)
    signature:  ay,   # Ed25519(priv, payload ‖ issued_at)
}
```

(Full interface in [04-plugin-ipc.md](./04-plugin-ipc.md).)

## Signed Structures

Both signatures derive from the **same in-memory Ed25519 keypair**.

### 1. Request envelope — `Overlay` (daemon → plugin, plugin verifies)

```
signature = Ed25519(private_key, payload ‖ issued_at)
```

- `payload` — the existing show/hide command variant (unchanged semantics).
- `issued_at` — `u64` unix milliseconds at sign time.
- Verified by the **plugin** with the public key from `DaemonPublicKey`. Proves
  the `Overlay` call came from the daemon's private key.

### 2. Echo-back token — `UserAction` (daemon → plugin → daemon, daemon verifies)

```
signature = Ed25519(private_key, app_id ‖ policy_id ‖ blocked_since ‖ instance_id)
```

- Rides inside the **show** command variant so the plugin can echo it back
  verbatim in `UserAction` alongside the user's `action` (the plugin's
  window-domain assertion, **not** signed).
- Verified by the **daemon** with its _own_ public key when `UserAction`
  arrives, before trusting `policy_id`. (`signature` field type `ay`.)
- `instance_id` binding means a token issued to plugin A cannot be replayed
  against plugin B.

The two signatures are independent: the outer envelope authenticates the
_request_ to the plugin; the inner token authenticates the _identifier_ the
plugin carries back to the daemon.

## Plugin-Side Verification (C++ / sdbus-cpp v2)

The plugin reads the public key **on demand** and verifies before dispatching.
For the canonical plugin structure (vtable registration, signal emission), see
[04-plugin-ipc.md](./04-plugin-ipc.md#c-plugin-side-sdbus-cpp-v2). The
verification function called from `handleOverlay` is:

```cpp
/// Verify the outer envelope of an Overlay call.
///
/// Checks:
///   1. Ed25519 signature over (payload_bytes ‖ issued_at_be) against
///      the daemon's public key (DaemonPublicKey property).
///   2. Timestamp freshness window (±30 s, stateless, no nonce cache).
bool verifyEnvelope(sdbus::IConnection& conn, const sdbus::Variant& payload,
                    uint64_t issuedAt, const std::vector<uint8_t>& sig) {
    // ── Freshness check (independent of crypto) ──────────────────────
    constexpr int64_t SKEW_MS = 30'000;
    const auto now = static_cast<int64_t>(
        std::chrono::duration_cast<std::chrono::milliseconds>(
            std::chrono::system_clock::now().time_since_epoch()).count());
    const auto ia = static_cast<int64_t>(issuedAt);
    if (ia < now - SKEW_MS || ia > now + SKEW_MS)
        return false;

    // ── Ed25519 verification (requires libsodium) ───────────────────
    auto [keyId, pubkey] = fetchDaemonPublicKey(connection);
    std::vector<uint8_t> msg = serializeVariant(payload);
    appendBE(msg, issuedAt);
    return crypto_sign_verify_detached(sig.data(), msg.data(), msg.size(),
                                       pubkey.data()) == 0;
}
```

`fetchDaemonPublicKey()` calls `org.wellbeing.v1.Daemon.DaemonPublicKey` over
the system bus. Because the daemon name is a protected well-known name owned
only by

`fetchDaemonPublicKey()` calls `org.wellbeing.v1.Daemon.DaemonPublicKey` over
the system bus. Because the daemon name is a protected well-known name owned
only by the system service, the public key fetched from it is the daemon's (same
root of trust the plugin already uses for `RegisterPlugin`).

## Daemon-Side Token Verification

When `UserAction` arrives, the daemon verifies the echoed token with its own
public key (same keypair that signed it):

```rust
impl PluginRegistry {
    /// Verify the echoed Ed25519 token before trusting `policy_id`.
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
        let sig = Signature::from_slice(&ev.signature).ok()?;
        if self.keypair.verifying_key().verify(&msg, &sig).is_err() {
            return None;   // impersonation / tampering -> drop
        }
        Some((AppId::new(ev.app_id.clone())?, ev.action, PolicyId::new(ev.policy_id)))
    }
}
```

On success `policy_id` is trusted as daemon-issued; the daemon re-derives the
policy config from its **own** DB by `policy_id`. The signature authenticates
the _identifier_ + instance binding; it never authenticates policy _values_.

## Restart & Replay Handling

- **Daemon restart.** The keypair regenerates; `key_id` changes. The plugin
  reads `DaemonPublicKey` fresh on every `Overlay`, so it always verifies
  against the current key. Pre-restart envelopes are naturally rejected (wrong
  key). No plugin-side key caching or `NameOwnerChanged` handling is required.
- **Plugin restart.** Normal case: the plugin re-fetches the key when it next
  needs to verify (which is immediately, on its first `Overlay` call).
- **Replay (stateless).** `issued_at` must fall in `[now − SKEW, now + SKEW]`
  (SKEW ≈ 30 s; local AF_UNIX, low latency). No nonce set is kept — `Overlay` is
  idempotent, so a within-window replay merely re-issues the same show/hide.

### Crash recovery (see [12-open-questions.md](./12-open-questions.md#3-daemon-crash-recovery-with-active-overlay))

On restart the daemon re-issues `Overlay(show)` for any app the plugin reports
with `overlay_shown == true` (via `CurrentSession`, which returns the same
`FocusVariant` as `FocusChanged` — see
[04-plugin-ipc.md](./04-plugin-ipc.md#per-app-multi-overlay-model)). The
re-issued envelope is signed with the **new** keypair and the plugin re-verifies
against the new public key it reads on demand. The user's next `UserAction`
click validates against the new key. No block state is restored or persisted by
the daemon.

## References

- [06-daemon-dbus.md](./06-daemon-dbus.md) — `org.wellbeing.v1.Daemon`,
  `DaemonPublicKey` property.
- [04-plugin-ipc.md](./04-plugin-ipc.md) — `org.wellbeing.v1.Manager`, `Overlay`
  envelope, multi-instance registration.
- [07-rbac.md](./07-rbac.md) — `SO_PEERCRED` uid authentication model.
- [12-open-questions.md](./12-open-questions.md) — crash recovery with active
  overlay (#3).
