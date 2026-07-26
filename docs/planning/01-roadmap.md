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

## Phase G — DNS-Level Domain Blocking

Block distracting websites at the DNS layer. The daemon runs a built-in DNS
forwarder on UDP :53, uses eBPF to correlate DNS queries with the originating
user's UID, and returns NXDOMAIN for blocked domains.

### G1 — eBPF UID correlation

- [ ] eBPF program attached to `udp_sendmsg` tracepoint. On each UDP send to
      destination port 53: - Call `bpf_get_current_uid_gid()` → get UID. - Read
      `sk_buff->source_port` → get ephemeral source port. - Write
      `map[source_port] = uid`.
- [ ] BPF map type: `BPF_MAP_TYPE_HASH`, key `u16` (source port), value `u32`
      (UID), max entries 65536.
- [ ] Load via `libbpf` or `aya` crate. Pinned to the daemon's lifecycle —
      loaded on daemon startup, unloaded on exit.

### G2 — DNS daemon

- [ ] UDP listener on `0.0.0.0:53` (tokio `UdpSocket`). - User configures
      `/etc/resolv.conf` → `nameserver 127.0.0.1`. - Daemon does NOT modify
      system configuration.
- [ ] On incoming query: 1. Parse DNS query header + question section (minimal
      parser — only extract QNAME and QTYPE). 2. Read source port from UDP
      header. 3. Lookup `uid = eBPF_map[source_port]`. 4. Query policy engine:
      is `domain` blocked for this `uid`? 5. Blocked → return `NXDOMAIN` (form
      `Status=3` response with the same TXID). 6. Allowed → forward to upstream
      DNS (`systemd-resolved` / stub), relay response back.
- [ ] DNS response cache (optional, v1: passthrough).
- [ ] Support UDP and TCP fallback (DNS over TCP for large responses).

### G3 — Policy integration

- [ ] `PolicyTarget::Domain(DomainPattern)` evaluated in first-match loop.
- [ ] `DomainPattern` supports: - Exact: `"reddit.com"` matches `reddit.com`. -
      Subdomain wildcard: `"*.reddit.com"` matches `old.reddit.com`. - Suffix:
      `".reddit.com"` matches everything under `reddit.com`. - Regex:
      `"/regex/"` for custom patterns (advanced).
- [ ] Policy applies to DNS resolution: if matched `Effect::Block`, the daemon
      returns NXDOMAIN. If `Effect::Allow`, the query is forwarded. If
      `Effect::TimeLimit`, the query is forwarded but the dashboard records the
      domain access (future: aggregate domain usage per day).

### G4 — Capabilities & deployment

- [ ] Daemon needs `CAP_NET_BIND_SERVICE` or root for port 53 (user's choice).
- [ ] `CAP_BPF` + `CAP_PERFMON` for eBPF.
- [ ] Document: DNS approach does not block DoH/DoT (apps with hardcoded DNS
      like Firefox's DoH). Users must disable DoH at the application level.
- [ ] eBPF program is optional — the daemon degrades gracefully: if eBPF fails
      to load, DNS queries from all UIDs are treated as the daemon's own user.
      (Single-user setups are unaffected.)

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
      `sql     INSERT OR IGNORE INTO domain_categories (domain, category_id, user_id) VALUES         ('reddit.com',  (SELECT id FROM categories WHERE name = 'Social'), 0),         ('twitter.com', (SELECT id FROM categories WHERE name = 'Social'), 0),         ('youtube.com', (SELECT id FROM categories WHERE name = 'Entertainment'), 0),         ('github.com',  (SELECT id FROM categories WHERE name = 'Development'), 0),         ('docs.rs',     (SELECT id FROM categories WHERE name = 'Development'), 0);     `
      The database is the single source of truth — no external config files.

### J2 — Domain-level categorization

- [ ] New table `domain_categories`:
      `sql     CREATE TABLE domain_categories (         domain      TEXT NOT NULL,         category_id INTEGER NOT NULL REFERENCES categories(id),         user_id     INTEGER NOT NULL DEFAULT 0,         PRIMARY KEY (domain, user_id)     );     `
- [ ] Resolution chain: 1. `domain_categories` user override
      (`user_id = uid`) 2. `domain_categories` system-global (`user_id = 0`) 3.
      AI classification (extends `AiClassifier` to domains) 4. Uncategorized

### J3 — AI classification extension

- [ ] Extend `AiClassifier` trait to accept domain names in addition to
      `app_class` and window titles.
- [ ] v1 heuristic: keyword matching (domain suffixes against category names).
- [ ] v2 burn model (Rust-native, no C++ deps), trained on domain + app_class.
- [ ] The DNS daemon queries the categorizer to resolve a domain's category
      before passing it to the policy engine.

### J4 — Policy integration

- [ ] `PolicyTarget::Domain` evaluation matches against the domain being
      resolved.
- [ ] DNS flow: `domain → category(ies) → matching policies → effect`.
- [ ] A domain can match via direct `PolicyTarget::Domain("reddit.com")` or via
      `PolicyTarget::Category(Social)` if the domain is categorized as Social.

---

## Phase K — Enhanced Reports

Builds on the existing reports pipeline (`GetUsageRange` → `DailySummary` →
charts).

- [ ] **Trend lines**: hours-per-category over 7/30/90 days (line chart overlay
      on bar chart).
- [ ] **Blocked attempt tracking**: counter in EnforcerActor records how many
      times each app was blocked per day. New API: `GetBlockedAttempts(range)`.
      Dashboard shows blocked-attempt bars alongside usage bars.
- [ ] **Drill-down**: click a category pie slice → filter to per-app breakdown
      within that category. (Already designed in F3-UI, not yet implemented.)
- [ ] **Domain usage**: if DNS daemon is active, record how many DNS queries
      were made per domain per day. Show "most-queried domains" alongside
      "most-used apps."
- [ ] **Export enhancements**: CSV/JSON exists; add optional PDF summary.

---

## Non-Goals (Never Part of This Project)

The following features from comparable tools are explicitly excluded:

| Feature                        | Rationale                                                  |
| ------------------------------ | ---------------------------------------------------------- |
| Cross-device / cloud sync      | Device-local only. No cloud, no account.                   |
| Mobile (iOS/Android)           | Linux desktop only.                                        |
| Focus sounds / ambient audio   | Out of scope. Use a dedicated app.                         |
| Session breaks / pause         | Locked mode is default — no bypass.                        |
| URL-level detection            | DNS-level domain blocking provides sufficient granularity. |
| MITM HTTPS proxy               | DNS+eBPF is simpler and less invasive.                     |
| Browser extensions             | Same functionality via DNS-level blocking.                 |
| Social features / leaderboards | Privacy-respecting by design.                              |
| Task / project management      | Digital wellbeing tool, not a planner.                     |

---

## Suggested Order of Attack

1. **Phase F** — Policy engine redesign (foundation for everything else).
2. **Phase G** — DNS daemon + eBPF (domain blocking).
3. **Phase H** — Allow-only mode (falls out of F, small GUI addition).
4. **Phase I** — DND integration (depends on H's `Block(Any)` detection).
5. **Phase J** — Preset blocklists + domain categorization (depends on G).
6. **Phase K** — Enhanced reports (independent, can start after F).
