# UI Design — gpui-component Screens

The GUI renders with gpui's retained-mode tree. All data arrives via the
in-memory cache documented in
[09-state-flow.md](../architecture/09-state-flow.md#gui-cache-architecture);
this file covers screen layout, components, and the view models that feed them.

## Screen Layout

The dashboard uses a TabBar to switch between three screens:

```text
┌─────────────────────────────────────────────────────────┐
│ ╔═══════════════════════════════════════════════════╗ │
│ ║ TitleBar                                            ║ │
│ ║  ⬤ Dashboard   ⬤ Policies   ⬤ Reports               ║ │
│ ╚═══════════════════════════════════════════════════╝ │
│                                                         │
│  ┌─────────────────────────────────────────────────────┐│
│  │                                                     ││
│  │             Screen content (varies by tab)          ││
│  │                                                     ││
│  └─────────────────────────────────────────────────────┘│
└─────────────────────────────────────────────────────────┘
```

## Component Mapping

| Screen    | Route            | Key Components                                  |
| --------- | ---------------- | ----------------------------------------------- |
| Dashboard | `Tab::Dashboard` | `BarChart`, `PieChart`, `AppList`, `BlockCard`  |
| Policies  | `Tab::Policies`  | `AppSelector`, `PolicyEditor`, `CategoryEditor` |
| Reports   | `Tab::Reports`   | `EventLog`, `ExportDialog`, `UsageTimeline`     |

## Dashboard Screen

Tab 0 — Daily usage overview.

### Time Range Selector

```rust
struct TimeRangeSelector {
    /// First day of the selected range.
    start: NaiveDate,
    /// Last day of the selected range (inclusive).
    end: NaiveDate,
}
```

Default is today. User can select Yesterday, This Week, Custom Range.

### Total Screen Time — BarChart

```rust
struct BarChart {
    /// One bar per day in the selected range.
    bars: Vec<Bar>,
    /// Max value for Y-axis scaling.
    max_minutes: f64,
}

struct Bar {
    date: NaiveDate,
    total_minutes: f64,
}
```

Renders as a simple column chart with day labels on X-axis and hours on Y-axis.

### Per-App Screen Time — PieChart

```rust
struct PieChart {
    slices: Vec<Slice>,
}

struct Slice {
    app_id: AppId,
    display_name: String,
    color: Color,
    percentage: f64,
}
```

Shows top N apps by usage in the selected date range. Click a slice → filter
other visualizations to that app.

### Per-Category Screen Time — PieChart

Same as per-app but grouped by `app_categories.category_id` → `categories.name`.

### Top 10 Apps — List / Table

```rust
struct AppList {
    entries: Vec<AppListEntry>,
}

struct AppListEntry {
    rank: usize,
    app_id: AppId,
    display_name: String,
    icon: Option<IconPath>,
    total_minutes: i64,
    percentage: f64,
    category_color: Option<Color>,
    is_blocked: bool,
}
```

Sortable by time (default), alphabetically, or by category.

### Currently Blocked Apps — Collapsible Card

```rust
struct BlockCard {
    app_id: AppId,
    display_name: String,
    blocked_since: SystemTime,
}
```

Renders as a collapsible read-only card when a block is active. Block resolution
is handled exclusively by the compositor overlay — the dashboard displays
information only:

```
┌──────────────────────────────────────────────────┐
│ ▼ Firefox — Blocked 12 minutes ago               │
│  Daily limit of 60m reached                      │
│  To continue using this app, switch to its       │
│  window and use the overlay controls.            │
└──────────────────────────────────────────────────┘
```

## Policies Screen

Tab 1 — Manage blocking and time-limit policies.

### App Selector — Input + Select Combination

```rust
struct AppSelector {
    apps: Vec<AppId>,
    selected: Option<AppId>,
    filter: String,
}
```

- **Input**: Free-text app_id entry (for apps not yet tracked).
- **Select**: Dropdown of all app_ids seen in the event log.
- Combined: Type to filter the dropdown, or type a new app_id and press Enter.

### Policy Configuration — Settings Pattern

The data layer constructs one of these per DB row — domain logic never sees raw
rows:

```rust
/// A policy targets exactly one thing. Constructed by PolicyRepository.
pub enum Policy {
    App { id: PolicyId, name: String, config: PolicyConfig, app_id: AppId },
    Category { id: PolicyId, name: String, config: PolicyConfig, category_id: CategoryId },
}

pub struct PolicyConfig {
    pub kind: PolicyKind,
    pub time_limit_seconds: Option<i64>,   // None for Block and Notify
    pub extra_seconds: i64,
    pub schedule: TimeWindow,
}

pub enum PolicyKind {
    Block,      // unconditional — always blocks when active, no time_limit_seconds
    TimeLimit,  // blocks when elapsed ≥ time_limit_seconds
    Notify,     // never blocks, alerts at threshold
}

struct TimeWindow {
    rules: Vec<ScheduleRule>,
}

enum ScheduleRule {
    Daily { start: LocalTime, end: LocalTime },
    Weekly { days: Vec<Weekday>, start: LocalTime, end: LocalTime },
}
```

**Construction rule:** `time_limit_seconds` is `Some` only for `TimeLimit` kind.
`Block` and `Notify` have it `None` (enforced by DB `CHECK` constraint).

Rendered as groups of labeled controls:

```text
Kind:         [block] [time_limit] [notify]
Target:       [App: firefox] or [Category: Entertainment]
Time limit:   [ 60 ] minutes (per day)
Extra time:   [ 10 ] minutes (extension grant)
Schedule:     [Every day] from [ 09:00 ] to [ 23:00 ]
              [Add rule]
Active:       [on/off]
```

## Responsive Layout Strategy

```rust
struct AppLayout {
    sidebar: Option<Column>,   // hidden on narrow screens
    main: Column,
    detail: Option<Column>,    // overlay or side panel
}

enum ViewportClass {
    Narrow,   // < 600px — stacked, single column
    Medium,   // 600-1000px — sidebar + main
    Wide,     // > 1000px — sidebar + main + detail
}
```

The layout switches between `Narrow` (phone-like), `Medium` (tablet), and `Wide`
(desktop) based on window width. On `Narrow`, the TabBar becomes a bottom
navigation bar; on `Medium` and `Wide`, it's a top tab bar.

## View Models

### DashboardViewModel

```rust
pub struct DashboardViewModel {
    pub date_range: (NaiveDate, NaiveDate),
    pub bar_chart: Vec<Bar>,
    pub pie_app: Vec<Slice>,
    pub pie_category: Vec<Slice>,
    pub top_apps: Vec<AppListEntry>,
    pub block_cards: Vec<BlockCard>,
}
```

### PoliciesViewModel

```rust
pub struct PoliciesViewModel {
    pub app_list: Vec<AppId>,
    /// Edit target: either an AppId or a CategoryId + its apps.
    pub selected_policy: Option<(PolicyTarget, PolicyConfig)>,
    pub categories: Vec<Category>,
    pub validation_errors: Vec<String>,
}

/// UI-level target selector — mirrors the Policy enum variants
/// but carries user-editable form data, not a finalized domain Policy.
pub enum PolicyTarget {
    App(AppId),
    Category(CategoryId),
}
```

## DB Query Patterns for Screens

See [persistence/01-database.md](../persistence/01-database.md) for schema
details. The queries below are the read paths each screen's view model is built
from.

### Dashboard — Daily Totals (same day range query)

```sql
-- Total screen time per day in the selected date range
SELECT date, SUM(total_seconds) as total_seconds
FROM daily_usage
WHERE date >= ? AND date <= ?
GROUP BY date
ORDER BY date;
```

### Dashboard — Per-App Breakdown

```sql
-- Per-app totals in the selected date range
SELECT du.app_id, du.total_seconds, du.extended,
       ao.display_name, ao.icon_path, c.name as category_name, c.color as category_color
FROM daily_usage du
LEFT JOIN app_categories ao ON du.app_id = ao.app_id
LEFT JOIN categories c ON ao.category_id = c.id
WHERE du.date >= ? AND du.date <= ?
ORDER BY du.total_seconds DESC
LIMIT 50;
```

### Dashboard — Per-Category Breakdown

```sql
-- Per-category totals in the selected date range
SELECT c.name, c.color, SUM(du.total_seconds) as total_seconds
FROM daily_usage du
JOIN app_categories ao ON du.app_id = ao.app_id
JOIN categories c ON ao.category_id = c.id
WHERE du.date >= ? AND du.date <= ?
GROUP BY c.id
ORDER BY total_seconds DESC;
```

### Policies — Distinct App IDs

```sql
-- All app_ids seen in events (for the policy screen's app selector)
SELECT DISTINCT app_id FROM events WHERE app_id IS NOT NULL ORDER BY app_id;
```

## Cross-Screen Navigation

```rust
enum NavigationEvent {
    NavigateToApp(String),
    NavigateToCategory(u32),
    NavigateToDate(NaiveDate),
}

impl DashboardViewModel {
    fn on_slice_click(&self, slice: &Slice) -> Option<NavigationEvent> {
        match slice {
            Slice::App(id, _) => Some(NavigationEvent::NavigateToApp(id.clone())),
            Slice::Category(cat_id, _) => Some(NavigationEvent::NavigateToCategory(*cat_id)),
        }
    }
}
```

Clicking a PieChart slice navigates to a filtered view showing detailed
breakdown for that app or category across the current date range.

## Implementation Modules

The dashboard is split across the two binaries, following the headless-daemon /
gui-client split. There is **no `ui/` directory inside the daemon's feature
directories** — gpui lives only in the `gui/` crate.

- **Daemon side** (`daemon/src/reports/`): `domain/` (aggregates, filter state,
  time range), `data/` (SQLite queries for dashboard totals), `core/` (report
  building). Results are exposed over D-Bus.
- **GUI side** (`gui/src/screens/dashboard/`): `mod.rs` + `view.rs`. The screen
  defines its **ViewModels** (above) from the D-Bus client + cache and renders
  gpui components (`BarChart`, `PieChart`, `AppList`, `BlockCard`).

```
gui/src/screens/dashboard/
├── mod.rs          # Screen registration, Tab::Dashboard route
└── view.rs         # DashboardViewModel + gpui component tree
```

## Future UI Enhancements

### 24h Timeline Strip Chart (v3)

A horizontal stacked bar showing 24 hours divided into 5-minute strips, colored
by app/category. Mousedown+mousemove reveals per-strip detail:

```
00:00 ████████████████████████████████████████████████████ 24:00
      ^--- Firefox 34m ---^ ^-- Code 12m --^
```

Not in v1. Referenced here to inform the data model — the `daily_usage` table
already captures per-app daily totals needed for this visualization.
