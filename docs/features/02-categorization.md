# App Categorization

Categorization is an **extra feature** layered on top of core app tracking. The
core function tracks **apps** — which apps you use and for how long. Categories
are a derived view: a grouping mechanism for policies and statistics.

## Data Sources

### 1. `app_categories` Table (Primary)

The single source of truth for app-to-category mappings. Built-in defaults are
seeded at migration time. Users modify entries via the UI settings screen —
there is no separate "override" concept, no external config files.

```sql
CREATE TABLE app_categories (
    app_id          TEXT NOT NULL,
    user_id         INTEGER NOT NULL DEFAULT 0,
    category_id     INTEGER REFERENCES categories(id) ON DELETE SET NULL,
    display_name    TEXT,
    icon_path       TEXT,
    ignore          INTEGER NOT NULL DEFAULT 0 CHECK(ignore IN (0, 1)),
    updated_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    PRIMARY KEY (app_id, user_id)
);
```

- `category_id` **nullable**: when NULL, the categorizer falls through to AI
  classification → `Uncategorized`.
- `display_name` / `icon_path`: optional overrides for UI presentation. Default
  is to show `app_id` as the display name with no icon.
- `ignore`: when set, the app is excluded from tracking and reports.

**Seeded defaults** (from the initial migration):

| Category     | App IDs                                                                                                                                   |
| ------------ | ----------------------------------------------------------------------------------------------------------------------------------------- |
| Productivity | Alacritty, kitty, foot, wezterm, gnome-terminal, konsole, terminator, tmux                                                                |
| Development  | Code, code-oss, zed, jetbrains-idea, nvim, emacs, Atom, Sublime_text                                                                      |
| Social       | firefox, firefoxdeveloperedition, Google-chrome, chromium-browser, brave-browser, zen-browser, org.mozilla.firefox, org.chromium.Chromium |

These are representative seed mappings. The complete set of built-in categories
seeded at migration is: Productivity, Communication, Entertainment, Social,
Development, Utilities, and Uncategorized (see Category System below).

New defaults are added via future migrations using `INSERT OR IGNORE` to
preserve user edits.

### 2. AI Classification (Fallback)

For apps not found in `app_categories`, the categorizer invokes a local AI
classification service. The classifier receives:

- `app_id` (e.g., `"Slack"`, `"obsidian"`, `"org.gimp.GIMP"`)
- Window `title` text when available (often contains app name)

And returns a `(WindowCategory, confidence)` pair.

**Integration boundary:**

```rust
pub trait AiClassifier: Send + Sync + 'static {
    /// Classify an app by app_id and optional window title.
    /// Returns None when confidence is below threshold (0.6 default).
    async fn classify(&self, app_id: &AppId, title: Option<&str>) -> Option<WindowCategory>;
}
```

**Implementation paths (v1 → v2):**

| Phase | Approach         | Details                                             |
| ----- | ---------------- | --------------------------------------------------- |
| v1    | Simple heuristic | Keyword matching on `app_id` against category names |
| v2    | ONNX local model | `ort` crate with a small distilled BERT model       |

The `AiClassifier` trait is **not** a `Platform` method — it lives in the
`categorization/` feature module alongside the `Categorizer` actor. This keeps
OS-specific concerns (D-Bus, plugin, metadata) fully isolated from the
classification logic.

**Cache:** Classification results are cached in an LRU
`HashMap<AppId, (WindowCategory, Instant)>` with a 60-second TTL.

### Resolution Chain

```
1. app_categories (user-specific, user_id=N) — per-user override
2. app_categories (system-global, user_id=0) — seeded default
3. AI classification — fallback for unmapped apps
4. Uncategorized — always succeeds (never crashes)
```

Each step is tried in order. The first match wins.

```rust
#[derive(Debug, Clone)]
pub enum CategorySource {
    AppCategory { app_id: AppId, category: WindowCategory },  // from DB
    AiClassified { app_id: AppId, category: WindowCategory },
    Uncategorized,
}

pub struct Categorizer<C: AiClassifier> {
    db: DbPool,
    ai: Arc<C>,
    cache: Mutex<LruCache<AppId, WindowCategory>>,
}

impl<C: AiClassifier> Categorizer<C> {
    pub async fn categorize(&self, app_id: &AppId, title: Option<&str>) -> CategorySource {
        // 1. DB lookup
        if let Some(cat) = self.lookup_db(app_id).await {
            return CategorySource::AppCategory { app_id: app_id.clone(), category: cat };
        }

        // 1b. Check cache (avoids repeated AI calls for same app)
        if let Some(cat) = self.cache.lock().get(app_id) {
            return CategorySource::AiClassified { app_id: app_id.clone(), category: *cat };
        }

        // 2. AI classification
        if let Some(cat) = self.ai.classify(app_id, title).await {
            self.cache.lock().put(app_id.clone(), cat);
            return CategorySource::AiClassified { app_id: app_id.clone(), category: cat };
        }

        // 3. Uncategorized
        CategorySource::Uncategorized
    }

    async fn lookup_db(&self, app_id: &AppId) -> Option<WindowCategory> {
        use diesel::prelude::*;
        app_categories::table
            .find(app_id.as_ref())
            .inner_join(categories::table)
            .select(categories::name)
            .first(&mut self.db.get().await.ok()?)
            .await
            .ok()
    }
}
```

## Cache Strategy

- DB lookups are fast (PK point query) — no separate cache needed.
- AI results cached in `LruCache<AppId, WindowCategory>` with 60-second TTL.
- Cache is invalidated on `app_categories` INSERT/UPDATE/DELETE via
  `ReactiveNotifier::PolicyMutated`.
- At startup, the cache is empty — AI classifies apps lazily on first focus.

### Cache Invalidation

When the user modifies an app's category in the UI, the change is written to
`app_categories` in the DB. The `ReactiveNotifier` broadcasts `PolicyMutated`,
which prompts the `Categorizer` to evict the cached entry for that `app_id`:

```rust
impl<C: AiClassifier> Categorizer<C> {
    /// Called when a PolicyMutated notification arrives.
    /// Removes the cached classification for the given app_id (if any).
    pub fn invalidate(&self, app_id: &AppId) {
        self.cache.lock().pop(app_id);
    }
}
```

## Category System

Categories are a secondary index. The system comes with built-in categories:

| Category      | Example apps                              |
| ------------- | ----------------------------------------- |
| Productivity  | Terminal, IDE, Office, Email              |
| Communication | Slack, Discord, Telegram, Element         |
| Entertainment | Games, Video players, Spotify             |
| Social        | Browser tabs (social media), Mastodon     |
| Development   | Docker, DB clients, CI/CD tools           |
| Utilities     | System settings, file manager, calculator |
| Uncategorized | Default bucket                            |

Built-in categories and default mappings are seeded in the migration (see
`migrations/`). Users can create custom categories via the settings UI.

### App-First, Categories-Second

```rust
/// Tracking is per-app. Categories are derived.
pub struct UsageRecord {
    pub app_id: AppId,
    pub duration: DurationSecs,
    pub category: Option<CategoryId>, // populated lazily
}
```

### Policy Domain Enum

The data layer constructs one of these per active policy row — domain logic
never sees raw `app_id`/`category_id` columns:

```rust
/// A policy targets exactly one scope. Enum variants encode the target
/// so the domain evaluator can pattern-match without peeking at Option fields.
pub enum Policy {
    App { id: PolicyId, name: String, config: PolicyConfig, app_id: AppId },
    Category { id: PolicyId, name: String, config: PolicyConfig, category_id: CategoryId },
}
```

If a policy targets a category, it applies to all apps in that category. The
evaluator (`evaluate()` in `policy/core/`) receives a `&[Policy]` pre-filtered
by the app's `app_id` and resolved `category_id`s — the DB query does the join,
not the domain.

Tracking data is always stored per-app — categories are computed at query time
via the `app_categories` table.

## Browser Tab Detection

Browser windows are a single `app_id` containing multiple "logical apps" (tabs).
Detected via window title patterns:

| Browser        | Pattern                     | Notes                                     |
| -------------- | --------------------------- | ----------------------------------------- |
| Firefox        | `^(.*) — .* Firefox$`       | Firefox uses em-dash (U+2014) globally    |
| Chromium       | `^(.*) - .* Google Chrome$` | Chromium uses regular hyphen, not em-dash |
| Chromium-based | `^(.*) - .* Chromium$`      | Same hyphen separator                     |

**Known locale sensitivity:** Firefox's title format is
`"<page> — Mozilla Firefox"` in all locales (the app name is not localized).
Chromium uses `"<page> - Google Chrome"` (regular hyphen, not em-dash). These
patterns are compiled statically with `Lazy<Regex>`. Brave, Edge, Vivaldi, and
Opera each have their own window title format and are not matched — they appear
as their base `app_id` (e.g., `brave-browser`). An `app_categories` entry is the
preferred classification path for less common browsers.

The captured group is the page title. URL detection is more invasive
(accessibility API), deferred until v2.

## Future: Machine Learning Classification

If manual categorization becomes a pain point, a local ML model (e.g., `ort` /
ONNX Runtime) could classify apps based on:

- `app_id` and window title patterns
- Usage time patterns (gaming = evening, work = morning)

The `AiClassifier` trait is designed for this — the v1 heuristic impl can be
swapped for an ONNX-based impl behind the same trait boundary.
