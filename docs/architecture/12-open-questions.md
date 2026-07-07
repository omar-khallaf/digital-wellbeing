# Open Questions

Open design questions. Items still under discussion are noted as open; decided
items state the chosen design.

1. **GUI startup when daemon is not running**: Should the GUI activate the
   daemon via systemd D-Bus activation, or should it show an error?

   **Decided** (see
   [13-deployment-modes.md](./13-deployment-modes.md#gui-daemon-resolution-client-side)
   and [#degraded-mode](./13-deployment-modes.md#degraded-mode)). The GUI
   resolves the daemon with a 4-step fallback: (1) system bus already has it →
   use it; (2) session bus already has it → use it; (3) `StartServiceByName` on
   the system bus (activates the root daemon) → use it; (4) `StartServiceByName`
   on the session bus (activates the user daemon) → use it. If all fail, the GUI
   shows a **warning banner** and opens in **degraded mode**
   (tracking/enforcement disabled, UI read-only) rather than erroring out. This
   covers both the root-installed and user-only install cases.

2. **Daemon crash recovery with active overlay.** The overlay is a plugin-owned
   state — the daemon keeps no in-memory block state (no `active_blocks` map).
   On restart the daemon's Ed25519 keypair is regenerated in memory and the new
   public key is published via the `DaemonPublicKey` property (see
   [05-daemon-auth.md](./05-daemon-auth.md)). The daemon queries
   `CurrentSession`; for any window reported with `overlay_shown == true`, it
   re-issues `Overlay(show)` wrapped in a freshly signed `SignedEnvelope`. The
   plugin re-verifies against the new public key it reads on demand, and the
   re-issued show also carries a fresh echo-back token. This refreshes both the
   request signature and the token embedded in the plugin's already-rendered
   overlay, so the user's next `UserAction` click validates against the new key
   (the keypair regenerates each start, so pre-restart envelopes/tokens are
   invalid and clicks during the gap are harmlessly dropped). When the user
   clicks, `UserAction` carries `app_id` + `action` (plugin authority) +
   `policy_id` + signature; the daemon verifies the Ed25519 signature and
   re-derives the policy from `policy_id` in its own DB. No block state is
   restored or persisted by the daemon.

3. **gpui-version compatibility**: The Cargo.toml references specific git
   branches of gpui/gpui-component/zeds-font-kit. These may have API changes.

   **Resolution:** Pin gpui and its companion crates to a specific git commit
   hash in `gui/Cargo.toml`, not to a branch name. Branch names move; commit
   hashes are immutable. Use a `cargo update` / dependabot workflow to advance
   the pinned commit:

   ```toml
   # gui/Cargo.toml — pin to known-good commit, not a branch
   gpui = { git = "https://github.com/zed-industries/zed", rev = "a1b2c3d4..." }
   gpui-component = { git = "https://github.com/zed-industries/zed", rev = "a1b2c3d4..." }
   ```

   The pin is updated only after verifying that the new revision compiles and
   the UI renders correctly. Tagged releases are preferred when available.

4. **Window-handle set tracking for multi-app blocking.** The per-app
   multi-overlay model (see
   [04-plugin-ipc.md](./04-plugin-ipc.md#per-app-multi-overlay-model)) requires
   the plugin to track a **set** of window handles per blocked `app_id`, not a
   single handle. The plugin populates these from compositor memory. This is
   currently stubbed — the window-handle set is empty and no compositor API call
   fills it. We need to decide: (a) which compositor API enumerates windows per
   `app_id`; (b) how the plugin observes window-close events to remove handles
   from the set; (c) whether handle geometry is needed for overlay positioning
   or if the overlay simply covers the full output.

   **Resolution:** Pending.

5. **Signal subscription in gpui main loop**: The background tokio thread
   receives D-Bus signals and pushes updates to gpui via mpsc. This means the
   gpui main loop must poll the mpsc channel each frame. Pattern:
   `cx.on_app_quit()` / `cx.spawn()` or a custom `Model` with a timer. Needs
   verification with the actual gpui API.
