# Roadmap

A versioned build plan. Phases build the system bottom-up and can proceed in
parallel where noted.

## Phase A — Foundation · `Done`

- [x] Workspace `Cargo.toml` with `core` / `daemon` / `gui` members.
- [x] `crates/core/*`: valuetypes, `Error`, `Clock`
      (+`SystemClock`/`VirtualClock`), D-Bus-flat domain types.
- [x] Initial schema with `user_id` / `created_by` / `owner_id` columns (RBAC
      scoping).
- [x] `crates/core/src/domain/policy_types.rs`: `PolicyKind` enum
      (`Block`/`TimeLimit`/`Notify`), `TimeWindow`, D-Bus types.

## Phase B — Daemon core · `Done`

- [x] `daemon/src/store/*`: `DbPool`, `StoreBuilder`, initial schema setup, WAL
      mode.
- [x] `daemon/src/platform/*`: `Platform` trait + `LinuxPlatform` +
      `ManagerClient` (system D-Bus, `NameOwnerChanged` discovery).
- [x] `daemon/src/dbus/mod.rs`: `org.wellbeing.v1.Controller` server + RBAC +
      `DaemonPublicKey` + `RegisterPlugin`.

## Phase C — Daemon actors · `Done`

- [x] `blocking/domain/`: `FocusState` domain type, daily-usage accumulation.
- [x] `policy/*`: `PolicyConfig` enum (`Block`/`TimeLimit`/`Notify`),
      `evaluate()`, `app_state()`, `TimeWindow`.
- [x] `categorization/*`: `Categorizer` + `AiClassifier` (v1 heuristic),
      `app_categories` chain.
- [x] `blocking/*`: `EnforcerActor` gate-first pipeline, `BlockingState`,
      `OverlayConfig`, plugin disconnect/reconnect.
- [x] `reports/*`: aggregate queries for history/export.
- [x] `main.rs`: wire `EnforcerActor` + event fan-out + D-Bus server +
      `logind::take_shutdown_inhibit` + SIGTERM/SIGINT handler.

## Phase D — GUI MVP · `Done`

- [x] `dbus/mod.rs`: `DaemonClient` zbus proxy + `SignalCoalescer`.
- [x] `cache/mod.rs`: `ClientCache<K,V>` stale-while-revalidate.
- [x] `main.rs`: `gpui::run` + background tokio thread + D-Bus activation
      fallback.
- [x] `app.rs`: app shell (TitleBar, TabBar, tray, Admin/User mode).
- [x] `dashboard/`: `DashboardViewModel`, `TimeRangeSelector`, usage charts.
- [x] `policies/`: `PoliciesViewModel`, `AppSelector`, `PolicyEditor`,
      `CategoryEditor`.
- [x] `reports/`: `ReportsViewModel`, `TimeRangeSelector`, export CSV/JSON.

## Phase E — Plugin + Deployment · `Done`

- [x] `plugins/hyprland/*`: `Event` signal, `CurrentFocus`, overlay rendering.
- [x] `deploy/*.conf`: D-Bus system policy files.
- [x] `deploy/systemd/digital-wellbeing-daemon.service`: systemd unit.

---

## Phase F — Policy Engine Redesign · `Done`

Replaces the ad-hoc "block what you name" model with a general-purpose rule
engine: priority-ordered, first-match-wins evaluation with explicit `Allow`
effect, `Notify` as non-terminating, and `Target::Any` wildcard. Schedules are
normalized into `policy_schedules` (day_mask bitmask, cross-midnight). The event
log is true append-only — Focus always written, Blocked(event_type=8)
terminates. Evaluation runs on every focus switch (prevents evasion) plus a
per-minute tick (eliminates per-app timers).

### F1 — Core types

- [x] New `Effect` enum (Allow, Block, TimeLimit, Notify) + `PolicyTarget` enum
      (App, Category, Domain, Any) + `priority` + `Vec<TimeWindow>` schedule.
- [x] `DomainPattern` newtype in `valuetypes.rs`. `TimeWindow` supports
      cross-midnight. D-Bus `PolicyData`/`PolicyInput` updated.

### F2 — Evaluation engine (`daemon/src/policy/core.rs`)

- [x] **`evaluate()` — pure domain fn**: priority-ordered, first-match-wins.
      `Allow`/`Block`/`TimeLimit` = terminating; `Notify` = non-terminating +
      collected; empty = unrestricted.
- [x] **Per-focus handler**: on every Focus event, `evaluate_and_enforce` runs
      immediately (upsert → query → evaluate → enforce verdict). Prevents
      evasion between minute-ticks.
- [x] **Per-minute tick**: re-evaluates the single focused app. Catches
      TimeLimit expiry during continuous use. Eliminates per-app timers.

### F3 — Database schema

- [x] `apps` registry, `policies` (normalized, no schedule_json),
      `policy_schedules` (day_mask bitmask, cross-midnight),
      `daily_usage_by_app` (FK to apps), `daily_usage_by_category`.
- [x] Event type 8 = Blocked. Old columns/schedule_json dropped.
      `app_categories` uses `app_id` FK; `daily_usage_by_title` keeps raw
      `app_class` TEXT (title granularity doesn't benefit from the FK).

### F4 — Evaluation query (push filtering into SQLite)

- [x] Hot-path SQL query with target + schedule filtering pushed down.
- [x] Rust eval loop: terminating/non-terminating split, domain policies
      excluded from app evaluation, upsert-before-select pattern.

### F5 — D-Bus updates

- [x] `ListPolicies` (sorted by priority), `CreatePolicy`/`UpdatePolicy` (new
      fields), `DeletePolicy` unchanged, `PolicyMutated` signal.

### F6 — EnforcerActor alignment

- [x] Focus always written first (append-only event log). Plugin decides tag=0
      (Focus) vs tag=2 (Block) based on BlockedApps.
- [x] `accumulate_daily_usage` upserts into all three tables. Focus→Blocked span
      counted; Blocked→next-Focus not.
- [x] `evaluate_and_enforce` uses new `evaluate()`. Block → update BlockedApps
      D-Bus property. Allow → no-op. Notify → one-shot `platform.notify()`.
- [x] Per-minute tick re-evaluates focused app. Old `PolicyVerdict`/`PolicyKind`
      types and per-app timers removed.

### F7 — CRUD data layer

- [x] `policy/data/queries.rs`: rewritten for new schema + sort by priority.
- [x] `filter_by_schedule` public helper accepts `&[TimeWindow]` → active bool.
- [x] Allow + TimeLimit on same priority permitted (first-match semantics).

---

## Phase G — D-Bus Interface Revamp & Browser Extension Domain Blocking

The daemon's D-Bus interface is redesigned: properties become methods, caller
uid is derived from `SO_PEERCRED` instead of message payloads, and signal names
are clarified. Domain blocking moves from the old DNS+eBPF approach to a
browser extension + native messaging bridge that mirrors the compositor plugin
model. The extension sends domain focus/unfocus events (chrome.tabs,
chrome.windows), the daemon tracks domain active time and evaluates domain
policies, and the extension enforces verdicts with tab-level overlays.

Two independent enforcement paths run in parallel:

| Layer | Overlay rendered by | Technology | Target |
|---|---|---|---|
| App-level | Compositor plugin | C++ OpenGL (Hyprland) | `app_class` (whole window) |
| Domain-level | Browser extension | HTML/CSS/JS (tab content) | `domain` (per tab) |

### G1 — D-Bus Interface Revamp (Breaking Change)

Interface `org.wellbeing.v1.Controller` is updated:

**Properties removed:**
- `BlockedApps` (property, read) — becomes `GetBlockedApps()` method.

**All methods derive caller uid from SO_PEERCRED — uid is never passed in the
message payload. Existing `user_id` / `uid` parameters are removed.**

**Policy CRUD (uid removed from all signatures):**
- `ListPolicies()` — returns policies for the calling user (was `ListPolicies(
  filter_owner)` with RBAC branching).
- `ListPoliciesForUser(uid)` — root only. Lists policies for any user.
- `CreatePolicy(input)` — `input.user_id` removed; owner uid derived from
  connection. Root can use `CreatePolicyForUser(input, target_uid)`.
- `CreatePolicyForUser(input, target_uid)` — root only. Creates a policy for
  another user.
- `UpdatePolicy(id, input)` — ownership check uses `created_by` from the row
  against connection uid. Root bypasses ownership check.
- `UpdatePolicyForUser(id, input, target_uid)` — root only. Updates any user's
  policy.
- `DeletePolicy(id)` — same ownership semantics as UpdatePolicy.
- `DeletePolicyForUser(id, target_uid)` — root only. Deletes any user's policy.

**Usage queries (uid removed from all signatures):**
- `GetUsageRange(start_date, end_date)` — returns usage summaries for the
  calling user (was `GetUsageRange(start, end, uid)`).
- `GetUsageRangeForUser(start_date, end_date, uid)` — root only. Returns usage
  for any user.

**Category methods (unrestricted, no uid change):**
- `ListCategories()` — unchanged (was already uid-free).
- `GetAppCategories()` — unchanged (owner uid derived from connection).
- `SetAppCategory(app_class, category_id)` — unchanged (owner uid derived from connection).

**Block/domain state (uid-free, caller-scoped):**
- `GetBlockedApps()` — returns blocked apps for the calling user (was
  `BlockedApps` property).
- `GetBlockedAppsForUser(uid)` — root only. Returns blocked apps for any user.
- `GetBlockedDomains()` — returns blocked domains for the calling user.
- `GetBlockedDomainsForUser(uid)` — root only. Returns blocked domains for any
  user.

**Client registration (reverse discovery):**
- `RegisterPlugin` — unchanged (compositor plugin registers itself; daemon
  learns uid via SO_PEERCRED, subscribes to the plugin's Event signal).
- `RegisterBridge` — new. Domain bridge registers itself following the same
  reverse-discovery pattern. Daemon learns uid via SO_PEERCRED, unique bus name
  from `header.sender()`, and subscribes to the bridge's `DomainEvent` signal.
  The bridge is a separate client type from the plugin — tracked independently
  in PluginRegistry (or a new BridgeRegistry).

**Signal renames:**
- `BlockedAppsChanged` → `AppBlocked` (payload unchanged: uid, app_class,
  blocked, reason).
- `PolicyMutated` → `PolicyChanged` (payload unchanged: uid).

**New signals:**
- `DomainBlocked(uid, domain, blocked, reason)` — emitted when a domain block
  is added or removed. Consumed by the bridge → forwarded to the extension.

**RBAC updates:**
- All methods derive caller uid from `SO_PEERCRED` (kernel-authenticated).
  Existing `user_id` / `uid` parameters removed from all method signatures.
- `*ForUser(uid)` methods return
  `org.freedesktop.DBus.Error.AccessDenied` for non-root callers. Validation
  order: **caller authentication** (SO_PEERCRED) → **ForUser check** (root
  required if explicit target_uid) → **domain authorization** (ownership,
  scope). This ordering ensures kernel-level identity is established before any
  business-logic check runs.

**Plugin contract updates (breaking — compositor plugin C++ code changes):**
- `BlockedApps` property read → `GetBlockedApps()` method call.
- `BlockedAppsChanged` → `AppBlocked` signal subscription.

### G2 — Domain Policy Evaluation & Domain Usage Tracking

- [ ] **DomainPattern suffix matching (reversed-prefix index):** Match semantics
      — `youtube.com` matches `youtube.com` (exact) and `www.youtube.com`
      (suffix) but not `xyoutube.com`. Implemented via the reversed-prefix
      trick: store `reverse(domain)` + `LIKE domain_pattern_rev || '.%'` for
      indexed queries. No regex support.
- [ ] **`daily_usage_by_domain` table:** Track per-domain active minutes per
      day, keyed by `(date, user_id, domain)`. Updated on the per-minute tick.
- [ ] **Domain usage accumulation:** In the per-minute tick, parallel to app
      usage accumulation, record active time for the currently focused domain.
- [ ] **`BlockedDomains` state:** New daemon state type mirroring
      `BlockedApps`. Tracks `{domain, policy_id, blocked_since, reason}`.
- [ ] **Domain policy evaluation:** Remove the
      `PolicyTarget::Domain(_) => false` stub in the evaluator. Implement full
      domain matching: for each domain event, query policies with
      `target_type = Domain`, match against `DomainPattern`, evaluate via the
      existing `evaluate()` function, update `BlockedDomains`, emit
      `DomainBlocked` signal.
- [ ] **Per-minute tick for domains:** Re-evaluate the focused domain
      (catches TimeLimit expiry during continuous browsing).
- [ ] **Cross-layer interaction:** App-level blocks (compositor) and
      domain-level blocks (extension) are independent. A domain policy
      evaluation never affects app-level state; an app block on the browser
      does not impact domain tracking within it.

### G3 — Native Bridge (Native Messaging Host)

The bridge is a peer to the compositor plugin, not a lightweight proxy. It
follows the same architectural pattern: connect to both D-Bus busses, register
with the daemon via reverse discovery, expose a D-Bus interface with signals,
and let the daemon subscribe.

- [ ] **Bridge binary (Rust):** A per-user process that runs as the same uid.
      Connects to both system and session D-Bus busses permanently (same 4-step
      resolution as the compositor plugin).
- [ ] **D-Bus interface (`org.wellbeing.v1.Bridge`):**
      - Exposes a `DomainEvent(tag: u32, domain: s, tab_id: u32)` signal
        (tag=0 Focus, tag=1 Unfocus, tag=2 Block). Mirrors the compositor
        plugin's `Event` signal but for domains.
      - Optionally exposes a `CurrentDomain` property (parallel to
        `CurrentFocus`) for crash recovery / startup sync.
      - Registered on both bus connections so the daemon can reach it from
        either bus.
- [ ] **Reverse registration:** At startup, the bridge calls `RegisterBridge()`
      on the daemon's `org.wellbeing.v1.Controller`. The daemon:
      1. Reads `SO_PEERCRED` uid from the connection.
      2. Reads the unique bus name from `header.sender()`.
      3. Creates a proxy to the bridge's interface.
      4. Subscribes to the `DomainEvent` signal stream.
- [ ] **Communication with extension (native messaging):**
      - Receives JSON messages from the extension on stdin (tab focus, unfocus,
        navigation, window focus change).
      - Translates each message into a `DomainEvent` D-Bus signal emission.
      - Subscribes to the daemon's `DomainBlocked` signal → writes JSON to
        stdout (read by the extension via the persistent native messaging
        port).
- [ ] **Lifecycle management:**
      - Daemon tracks the bridge instance in BridgeRegistry (parallel to
        PluginRegistry), watching `NameOwnerChanged` for the bridge's unique
        bus name to detect crash/disconnect.
      - On daemon disconnect/reconnect: bridge re-runs 4-step resolution,
        re-calls `RegisterBridge`, re-subscribes to `DomainBlocked`.
      - On bridge crash: daemon cleans up any pending domain state for that
        uid. When the bridge reappears, full re-registration restores tracking.
- [ ] **Native messaging manifest:** JSON deployed to the browser's native
      messaging hosts directory (e.g. `~/.mozilla/native-messaging-hosts/` or
      `/etc/opt/chrome/native-messaging-hosts/`). Maps
      `com.wellbeing.bridge` to the bridge binary path.
- [ ] **Lifecycle:** The browser auto-starts the bridge on first
      `runtime.connectNative`. The bridge exits when the browser closes the
      port. Single instance per user.

### G4 — Browser Extension (MV3 + Firefox)

- [ ] **Manifest V3 extension** (Chrome/Edge/Chromium) + **Firefox variant**
      (identical logic, `browser.*` namespace).
- [ ] **Native messaging connection:** `browser.runtime.connectNative(
      "com.wellbeing.bridge")` — persistent port for bidirectional
      communication.
- [ ] **Tab/window focus tracking (event source):**
      - `chrome.tabs.onActivated` → `SubmitDomainEvent(Focus, domain, tab_id)`
      - `chrome.tabs.onUpdated` (navigation) → `SubmitDomainEvent(Unfocus,
        old_domain, tab_id)` + `SubmitDomainEvent(Focus, new_domain, tab_id)`
      - `chrome.tabs.onRemoved` (if was focused) →
        `SubmitDomainEvent(Unfocus, domain, tab_id)`
      - `chrome.windows.onFocusChanged` (`WINDOW_ID_NONE`) →
        `SubmitDomainEvent(Unfocus, current_domain, tab_id)`
      - `chrome.windows.onFocusChanged` (window regained) →
        `SubmitDomainEvent(Focus, active_tab_domain, tab_id)`
- [ ] **Block enforcement:** Receives `DomainBlocked` notifications from the
      bridge. When a domain is blocked, replaces the tab content with a
      blocking overlay (HTML/CSS block page with reason, time remaining for
      TimeLimit). When unblocked, reloads the original page or removes the
      overlay.
- [ ] **Graceful degradation:** If the bridge is not running (no native
      messaging port), the extension operates in passive mode — no tracking,
      no blocking. Shows a status indicator.

### G5 — Policy & GUI Integration

- [ ] **Domain policy CRUD:** Already supported in Phase F (Target selector →
      Domain, `PolicyData.domain_pattern`). No GUI changes needed for creation.
      The policy list already renders `PolicyTarget::Domain`.
- [ ] **Dashboard blocked domains card:** New card showing currently blocked
      domains (parallel to the blocked apps card). Reads `GetBlockedDomains()`.
- [ ] **Reports domain usage:** Per-domain usage breakdown (parallel to per-app
      usage). Reads from `daily_usage_by_domain`.
- [ ] **Bridge connection status:** Dashboard shows a status indicator when the
      bridge is disconnected (extension not enforcing domain blocks).
- [ ] **Domain pattern input validation:** GUI validates `DomainPattern` on
      input (no regex, plain domain/suffix format only).

---

## Phase H — Allow-Only (Deep Work) Mode

Allow-only is not a separate toggle — it falls out of the policy model
naturally:

```
priority=100: Block(Any)     ← catch-all: block everything
priority= 10: Allow(Dev)     ← exception: development tools pass
priority= 20: Allow(Firefox) ← exception: browser passes
priority= 30: TimeLimit(Firefox, 30)
```

- [ ] GUI: a prominent "Deep Work" toggle that creates/deletes the `Block(Any)`
      policy. When active, the interface switches from "what to block" to "what
      to allow."
- [ ] Allow-only is just `Block(Any)` at low priority + `Allow(...)` targets at
      high priority. No special UI state machine.

---

## Phase I — DND Integration

Do Not Disturb activates automatically when `Block(Any)` is active (i.e., the
user has entered allow-only/deep-work mode).

- [ ] `Platform::set_dnd(bool)` — Linux impl sends
      `org.freedesktop.Notifications.Inhibit` D-Bus call.
- [ ] MockPlatform: no-op.
- [ ] EnforcerActor: when `Block(Any)` policy is present and active, call
      `platform.set_dnd(true)`. When no `Block(Any)` policy exists, call
      `platform.set_dnd(false)`.
- [ ] On daemon startup, reconcile DND state against active policies.

---

## Phase J — Preset Blocklists & Domain Categorization

### J1 — Curated domain dataset

- [ ] Default domain-to-category mappings are seeded in the initial migration,
      same pattern as `app_categories`:
      ```sql
      INSERT OR IGNORE INTO domain_categories (domain, category_id, user_id) VALUES
          ('reddit.com',  (SELECT id FROM categories WHERE name = 'Social'), 0),
          ('twitter.com', (SELECT id FROM categories WHERE name = 'Social'), 0),
          ('youtube.com', (SELECT id FROM categories WHERE name = 'Entertainment'), 0),
          ('github.com',  (SELECT id FROM categories WHERE name = 'Development'), 0),
          ('docs.rs',     (SELECT id FROM categories WHERE name = 'Development'), 0);
      ```
      The database is the single source of truth — no external config files.

### J2 — Domain-level categorization

- [ ] New table `domain_categories`:
      ```sql
      CREATE TABLE domain_categories (
          domain      TEXT NOT NULL,
          category_id INTEGER NOT NULL REFERENCES categories(id),
          user_id     INTEGER NOT NULL DEFAULT 0,
          PRIMARY KEY (domain, user_id)
      );
      ```
- [ ] Resolution chain: 1. `domain_categories` user override
      (`user_id = uid`) 2. `domain_categories` system-global (`user_id = 0`) 3.
      AI classification (extends `AiClassifier` to domains) 4. Uncategorized

### J3 — AI classification extension

- [ ] Extend `AiClassifier` trait to accept domain names in addition to
      `app_class` and window titles.
- [ ] v1 heuristic: keyword matching (domain suffixes against category names).
- [ ] v2 burn model (Rust-native, no C++ deps), trained on domain + app_class.
- [ ] Domain categorization feeds into policy evaluation: a domain policy
      targeting a Category matches all domains in that category.

### J4 — Policy integration

- [ ] `PolicyTarget::Domain` evaluation uses the reversed-prefix matching
      against the domain being tracked.
- [ ] Domain flow: `domain → category(ies) → matching policies → effect`.
- [ ] A domain can match via direct `PolicyTarget::Domain("reddit.com")` or via
      `PolicyTarget::Category(Social)` if the domain is categorized as Social.

---

## Phase K — Enhanced Reports & Timeline

Builds on the existing reports pipeline (`GetUsageRange` → `DailySummary` →
charts). Expands the visualization layer with interactive timeline components
and cross-chart drill-down.

- [ ] **Trend lines**: hours-per-category over 7/30/90 days (line chart overlay
      on bar chart).
- [ ] **Blocked attempt tracking**: counter in EnforcerActor records how many
      times each app was blocked per day. New API: `GetBlockedAttempts(range)`.
      Dashboard shows blocked-attempt bars alongside usage bars.
- [ ] **Drill-down**: click a category pie slice → filter to per-app breakdown
      within that category. (Already designed in F3-UI, not yet implemented.)
- [ ] **Domain usage**: record domain focus events per day (from extension
      tracking). Show "most-used domains" alongside "most-used apps."
- [ ] **Export enhancements**: CSV/JSON exists; add optional PDF summary.
- [ ] **Timeline component**: replaces the horizontal day bar with a vertical
      row-per-day layout. Each day is a row containing interval bars showing
      start/end times of focused sessions. Multi-day views stack rows
      vertically in a scrollable container.
  - Dashboard zoomed day view: single row with all intervals for that day.
  - Reports multi-day view: one row per day in the selected date range,
    scrollable. Adaptive granularity (per-day for 7d, per-week for 30d,
    per-month for 90d).
- [ ] **Clickable intervals (cross-chart highlighting):** clicking an interval
      in the timeline:
  - Highlights all intervals that fall within the same time span (same bar in
    the row).
  - Highlights the corresponding app section in the per-app pie chart (usage
    that occurred during the selected interval).
  - Shared `SelectedInterval` state between timeline and chart components.
- [ ] **Cross-filter sync:** clicking a pie slice filters the timeline to show
      only intervals for that app/category. Clicking a timeline interval
      highlights the relevant pie slices. Bidirectional.

---

## Non-Goals (Never Part of This Project)

The following features from comparable tools are explicitly excluded:

| Feature                        | Rationale                                                  |
| ------------------------------ | ---------------------------------------------------------- |
| Cross-device / cloud sync      | Device-local only. No cloud, no account.                   |
| Mobile (iOS/Android)           | Linux desktop only.                                        |
| Focus sounds / ambient audio   | Out of scope. Use a dedicated app.                         |
| Session breaks / pause         | Locked mode is default — no bypass.                        |
| MITM HTTPS proxy               | Unnecessary — browser extension handles domain blocking.   |
| Social features / leaderboards | Privacy-respecting by design.                              |
| Task / project management      | Digital wellbeing tool, not a planner.                     |

---

## Suggested Order of Attack

1. **Phase F** — Policy engine redesign (foundation for everything else). DONE.
2. **Phase G1** — D-Bus interface revamp (prerequisite: all downstream phases
   depend on the new method-based, uid-free interface).
3. **Phase G2–G5** — Domain policy evaluation, bridge, extension, GUI
   integration (can proceed sequentially within G).
4. **Phase H** — Allow-only mode (small GUI addition on top of Phase F).
5. **Phase I** — DND integration (depends on H's `Block(Any)` detection).
6. **Phase J** — Preset blocklists + domain categorization (domain data, feeds
   into policy matching from G2).
7. **Phase K** — Enhanced reports + timeline (independent; can start after G1's
   D-Bus interface stabilizes or even in parallel after F).
