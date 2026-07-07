# Roadmap (Detailed)

A versioned build plan derived from the design docs in `docs/`. Module paths,
D-Bus interfaces, and source docs are cited so any task can be picked up without
re-deriving scope.

**Status** (shown once per section, not per line): `Done` · `In progress` ·
`Ready` (designed, implementable) · `Open` (design question outstanding — see
`architecture/12-open-questions.md`).

**Engineering gates (every task):** valuetype boundary gate for raw
`String`/`i32`/`bool` (`core/valuetypes.rs`); `thiserror` domain errors, no
formatted strings (`core/error.rs`); `Clock` trait for time-dependent logic
(`core/clock.rs`); zero-alloc hot path + `#[inline]`; `unsafe` only with
`// SAFETY:` + review; test behavior not structure, assert both domain events
and DB state, mock only at boundaries (`quality/02-testing.md`).

---

## Phases — infrastructure first

The phases build the system bottom-up. Versions (below) are the product
milestones that the phases deliver.

### Phase A — Foundation · `Done`

- [x] Workspace `Cargo.toml` with `core` / `daemon` / `gui` members.
- [x] `crates/core/*`: valuetypes, `Error`, `Clock`
      (+`SystemClock`/`VirtualClock`), D-Bus-flat domain types.
- [x] Initial schema with `user_id` / `created_by` / `owner_id` columns (RBAC
      scoping — `architecture/07-rbac.md`).

### Phase B — Daemon core · `In progress`

- [x] `daemon/src/store/*`: `DbPool`, `StoreBuilder`, initial schema setup, WAL
      mode (`persistence/01-database.md`).
- [ ] `daemon/src/platform/*`: `Platform` trait + `LinuxPlatform` +
      `ManagerClient` (system D-Bus, `NameOwnerChanged` discovery —
      `architecture/02-platform.md`, `architecture/04-plugin-ipc.md`).
- [ ] `daemon/src/dbus/mod.rs`: `org.wellbeing.v1.Daemon` server + RBAC +
      `DaemonPublicKey` + `RegisterPlugin` (`architecture/06-daemon-dbus.md`,
      `architecture/05-daemon-auth.md`).

### Phase C — Daemon actors · `Ready`

- [ ] `tracking/*`: `TrackerActor`, `HashMap<Uid, FocusState>`,
      `accumulate_interval()`.
- [ ] `policy/*`: `PolicyEngine.evaluate()`, domain types, `TimeWindow`.
- [ ] `categorization/*`: `Categorizer` + `AiClassifier` (v1 heuristic),
      `app_categories` chain.
- [ ] `blocking/*`: `EnforcerActor` gate-first pipeline, `BlockingState`,
      `OverlayConfig`, grant-extension, plugin disconnect/reconnect
      (`features/01-blocking.md`).
- [ ] `reports/*`: aggregate queries for history/export.
- [ ] `main.rs`: wire actors + D-Bus server + `ReactiveNotifier` +
      `PowerStateWatcher` + SIGTERM/SIGHUP handler + startup reconciliation.

### Phase D — GUI · `Ready`

- [ ] `dbus/mod.rs`: `DaemonClient` zbus proxy + `SignalCoalescer`, error
      mapping, subscribe to daemon signals (`architecture/06-daemon-dbus.md`,
      `architecture/09-state-flow.md`).
- [ ] `cache/mod.rs`: `ClientCache<K,V>` stale-while-revalidate, cache
      invalidation from daemon signals (`architecture/09-state-flow.md`).
- [ ] `main.rs`: `gpui::run` + background tokio thread + D-Bus activation
      fallback.
- [ ] `app.rs`: app shell (TitleBar, TabBar, tray, Admin/User mode via
      `getuid()`).
- [ ] `screens/dashboard/`: `DashboardViewModel`, `BarChart`, `PieChart`×2,
      `AppList`, `BlockCard` (`features/03-ui-design.md`).
- [ ] `screens/policies/`: `PoliciesViewModel`, `AppSelector`, `PolicyEditor`,
      `CategoryEditor` (RBAC-aware).
- [ ] `screens/reports/`: history + export (stub now, built out in v3).

### Phase E — Plugin migration · `Ready`

- [ ] `plugins/hyprland/*`: session bus → **system bus**; add `CurrentSession`;
      `RegisterPlugin(instance_id)` reverse discovery; `ActivityChanged` idle
      signal; verify `SignedEnvelope` (read `DaemonPublicKey`, ±30s skew —
      `features/01-blocking.md`, `architecture/05-daemon-auth.md`).
- [ ] `deploy/*.conf`: D-Bus system policy files for both interfaces
      (`architecture/10-deployment.md`).

### Phase F — Deployment · `Ready`

- [ ] `deploy/digital-wellbeing-daemon.service`: systemd unit (`Type=dbus`,
      root, hardening, `StateDirectory`).
- [ ] `deploy/*.service`: D-Bus activation + `Makefile`/`justfile` install
      targets.

---

## v1 — Core Digital Wellbeing (current target)

Single compositor (Hyprland), full tracking → policy → block → dashboard loop.

### Tracking · `Ready`

1. **Hyprland `FocusChanged`** — C++ `wellbeing-lockdown.so`, sdbus-cpp,
   `RENDER_PASS_POST_WINDOW` hook →
   `WindowInfo{app_id,title,pid,uid, overlay_shown}` (`features/01-blocking.md`,
   `architecture/04-plugin-ipc.md`).
2. **Event-driven usage** — `TrackerActor` writes one append-only `events` row
   per `WindowFocused`/`Unfocused`; `accumulate_interval()` updates
   `daily_usage` in the same transaction (`persistence/01-database.md`).
3. **Idle/Resume + power/session closes** — `ActivityChanged` →
   `Idle`/`Resumed`; `PowerStateWatcher` (logind) → real
   `Slept`/`ShutDown`/`Locked`/`LoggedOut`; SIGTERM/SIGINT → `LoggedOut`
   (`architecture/03-linux-platform.md`).

### Enforcement · `Ready`

1. **One-shot per-app timer (no polling)** — `EnforcerActor` spawns
   `tokio::sleep(remaining)` on focus; re-evaluates on expiry; cancels on switch
   (`features/01-blocking.md`).
2. **Policy engine** — pure `evaluate(app_id, &[Policy], elapsed, now)` with AND
   semantics; `Block`/`TimeLimit`/`Notify`; `extra_seconds`; `TimeWindow`
   (`features/01-blocking.md`, `features/02-categorization.md`).
3. **Overlay-only blocking** — gate-first evaluate before DB write; blocked app
   never logged; `grant_extension()` writes synthetic `WindowFocused` +
   `extended`; no in-memory block state (`features/01-blocking.md`).
4. **Block overlay (plugin)** — OpenGL backdrop + `Extra`/`Close` buttons; traps
   input; `UserAction` carries plugin `app_id`+`action` and daemon-signed
   `policy_id` token (`features/01-blocking.md`).

### UI · `Ready`

1. **Settings panel** — `Policies` tab: `AppSelector`, `PolicyEditor`,
   `CategoryEditor`, RBAC read-only badges (`features/03-ui-design.md`).
2. **Dashboard** — `Dashboard` tab: `BarChart`, `PieChart`×2, `AppList` top-10,
   `BlockCard` (`features/03-ui-design.md`).

### Persistence & state · `Ready`

1. **SQLite (WAL)** — `events` (generated cols + CHECK JSON), `daily_usage`,
   `policies` (exclusive-arc + kind CHECKs), `categories`, `app_categories`;
   initial schema (`persistence/01-database.md`).
2. **ReactiveNotifier → signals** — `watch` channel drives `BlockStateChanged` /
   `DailyUsageChanged` / `PolicyMutated` (`architecture/09-state-flow.md`).
3. **Seeded `app_categories`** — built-in categories + `INSERT OR IGNORE`
   defaults replace `.desktop`/config parsing (`features/02-categorization.md`).

### Categorization (v1) · `Ready`

1. **AI v1 heuristic** — `AiClassifier` trait, resolution chain
   `app_categories → AI → Uncategorized`, LRU cache (60s)
   (`features/02-categorization.md`).
2. **User category edits** — `SetAppCategory` + settings row; `ignore` excludes
   from tracking.

### Real-time UI plumbing · `Ready`

1. **Minimal watch channels** — block state (`BlockStateChanged`) and
   invalidation signals (`DailyUsageChanged`, `PolicyMutated`); `ClientCache`
   TTLs: signal-drive for block states, 500ms usage, 5s policies
   (`architecture/09-state-flow.md`).

### v1 hardening · `Open`

- [ ] **Crash recovery with active overlay** — re-issue `Overlay(show)` with
      fresh token when plugin reports `overlay_shown==true`
      (`12-open-questions.md#3`, `features/01-blocking.md`).
- [ ] **GUI startup when daemon down** — D-Bus activation vs error dialog
      (`12-open-questions.md#2`).
- [ ] **gpui version pin** — commit hash, not branch; advance via dependabot
      after verification (`12-open-questions.md#4`).
- [ ] **Signal subscription in gpui loop** — mpsc poll vs `cx.spawn()` timer
      (`12-open-questions.md#5`).

---

## v2 — Additional Compositors · `Ready`

Each compositor is a new plugin under `plugins/<name>/` speaking the **same**
`org.wellbeing.v1.Manager` contract. **No daemon/tracker/policy/UI changes** —
discovery (`RegisterPlugin` + `NameOwnerChanged`) and uid-routed overlays
already exist (`architecture/03-linux-platform.md`).

1. **KWin** — `wellbeing-effect` (`KWin::Effect` + D-Bus).
2. **Wayfire** — `wellbeing-plugin` (Wayfire API + D-Bus).
3. **GNOME Shell** — `wellbeing-extension` (GJS + D-Bus, verifies
   `SignedEnvelope`).
4. **Shared extension template** — `LockManager`/render-hook/input-trap adapter
   over the D-Bus contract.

Per compositor deliverable: plugin binary + README + D-Bus policy entry + CI
build.

---

## v3 — Statistics & History · `Ready`

1. **Reports panel** — daily/weekly/monthly via `GetUsageRange` + time-range
   selector (`features/03-ui-design.md`, `architecture/06-daemon-dbus.md`).
2. **Usage trends** — hours-per-category over time.
3. **24h timeline strip** — custom gpui element (no built-in timeline); respects
   `EVENTS_RETENTION_DAYS` (`features/03-ui-design.md`,
   `persistence/01-database.md`).
4. **Export CSV/JSON** — `reports/` core + `ExportDialog`.
5. **Drill-down** — per-app within category via `NavigationEvent`
   (`features/03-ui-design.md`).

---

## v4 — Advanced Classification · `Ready`

1. **Browser tab URL detection** — accessibility API (keyword title heuristics
   stay as fallback) (`features/02-categorization.md`).
2. **Domain categorization** — social / news / work by domain.
3. **Custom categories in UI** — `categories` insert + `CategoryEditor`; seeds
   preserved via `INSERT OR IGNORE`.
4. **Local ML (ONNX)** — swap `AiClassifier` heuristic for `ort` + distilled
   BERT behind the same trait (`features/02-categorization.md`).

---

## v5 — TUI (Deferred) · `Open`

1. **ratatui terminal UI** for headless/SSH — separate binary under a `tui`
   feature gate; reuses `DaemonClient` + `ClientCache`, same `ViewModel`→render
   split. Deferred: second UI framework, a11y/input/terminal-detect surface.

---

## v6 — Integration API (D-Bus only) · `Ready`

External apps query daemon state over the existing system D-Bus interface; no
separate transport is added.

1. **Read-only query API** — current usage / policies / history exposed via the
   `org.wellbeing.v1.Daemon` method set (mirrors `GetDailyUsage` /
   `GetUsageRange` / `ListPolicies`); no writes.
2. **Command API** — `toggle block`, `grant extension` as new D-Bus methods on
   `org.wellbeing.v1.Daemon` (same RBAC + `SO_PEERCRED` uid check as existing
   methods; reuses `EnforcerActor` path).

Constraint: D-Bus is the single integration surface — external apps own their
own sync/CRDT/event-sourcing layer; the daemon never exposes a second transport
(`architecture/07-rbac.md`).

---

## Non-Goals

Never part of this project: task/project management · calendar integration ·
study notes/flashcards · cross-device sync · cloud backup · social features
(leaderboards, sharing, competitive focus).

---

## Suggested order of attack

1. Finish **Phase B2–B3** (platform + D-Bus server) → unblocks all actors.
2. **Phase C** in order: tracking → policy → categorization → blocking → reports
   → main.rs wiring (incl. `PowerStateWatcher` + signals + startup
   reconciliation).
3. **Phase E** plugin system-bus + `CurrentSession` + signed overlays (security-
   critical; pair with `architecture/05-daemon-auth.md` tests).
4. **Phase D** GUI (dashboard → policies → reports stub).
5. **Phase F** packaging + D-Bus policy + systemd.
6. Resolve v1 **Open** items before tagging v1.
7. v2+ compositor-by-compositor, then v3 analytics, v4 classification, v6 API;
   v5 only on demand.
