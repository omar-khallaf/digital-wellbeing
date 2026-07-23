# App Categorization

Categorization is an extra feature layered on top of core app tracking. The core
function tracks apps — which apps you use and for how long. Categories are a
derived view: a grouping mechanism for policies and statistics.

## Data Sources

### 1. app_categories Table (Primary)

The single source of truth for app-to-category mappings. Built-in defaults are
seeded at migration time. Users modify entries via the UI settings screen —
there is no separate "override" concept, no external config files.

The table stores one row per app per user. Each row carries the app identifier,
a user_id (0 for system-global defaults, the caller's UID for per-user
overrides), a nullable category_id that references the categories table (null
means the categorizer falls through to AI classification then Uncategorized), an
optional display name for UI presentation, an optional icon path, an ignore flag
that excludes the app from tracking and reports when set, and an updated_at
timestamp.

The primary key is (app_id, user_id), which allows each user to override their
own mappings while falling through to the system-global row at user_id=0.

Seeded defaults (from the initial migration) assign common apps to built-in
categories: terminal emulators and tmux to Productivity; editors and IDEs to
Development; browsers to Social. Other built-in categories include
Communication, Entertainment, and Utilities, with Uncategorized as the default
bucket. The complete set of built-in categories is seeded at migration time.

New defaults are added via future migrations using INSERT OR IGNORE to preserve
user edits.

### 2. AI Classification (Fallback)

For apps not found in app_categories, the categorizer invokes a local AI
classification service. The classifier receives the app_id (e.g., "Slack",
"obsidian", "org.gimp.GIMP") and window title text when available (often
contains app name). It returns a (WindowCategory, confidence) pair.

The classification logic is expressed as a single async method: classify, which
accepts an AppId and an optional window title, and returns
Option<WindowCategory>. It returns None when confidence falls below the default
threshold of 0.6.

Implementation paths:

- v1: simple heuristic — keyword matching on app_id against category names
- v2: ONNX local model — ort crate with a small distilled BERT model

The AiClassifier trait is not a Platform method — it lives in the
categorization/ feature module alongside the Categorizer actor. This keeps
OS-specific concerns (D-Bus, plugin, metadata) fully isolated from the
classification logic.

Cache: classification results are cached in an LRU HashMap<AppId,
(WindowCategory, Instant)> with a 60-second TTL.

### Resolution Chain

The categorizer tries each step in order; the first match wins:

1. app_categories (user-specific, user_id=N) — per-user override
2. app_categories (system-global, user_id=0) — seeded default
3. AI classification — fallback for unmapped apps
4. Uncategorized — always succeeds (never crashes)

The categorizer tracks which source produced each result via a CategorySource
enum with three variants:

- AppCategory — the result came from the app_categories DB table, carrying the
  app_id and resolved WindowCategory
- AiClassified — the result came from AI classification, carrying the app_id and
  WindowCategory
- Uncategorized — no mapping was found; the fallback bucket

Internally, the categorizer holds a reference to the database pool and a
thread-safe LRU cache keyed by AppId. When categorize(app_id, title) is called,
it performs this sequence:

1. DB lookup queries app_categories joined with categories. If a matching row
   has a non-null category_id, the resolved category is returned immediately as
   AppCategory.
2. Cache check — if the cache already holds a result for this app_id, it is
   returned as AiClassified without invoking the AI model again.
3. AI classification calls the AiClassifier. If the model returns a category
   above the confidence threshold, the result is cached and returned as
   AiClassified.
4. Uncategorized — if all prior steps yield nothing, the result is
   Uncategorized.

The DB lookup performs a parameterized query using app_id as the key with an
INNER JOIN on categories. It tries the caller's user_id first, then falls
through to user_id=0 for the system-wide default. If neither row exists or both
have category_id IS NULL, the lookup returns none and the categorizer advances
to the next step.

## Cache Strategy

- DB lookups are fast (PK point query) — no separate cache needed.
- AI results cached in LruCache<AppId, WindowCategory> with 60-second TTL.
- Cache is invalidated on app_categories INSERT/UPDATE/DELETE via PolicyMutated.
- At startup, the cache is empty — AI classifies apps lazily on first focus.

### Cache Invalidation

When the user modifies an app's category in the UI, the change is written to
app_categories in the DB. The daemon broadcasts PolicyMutated, which prompts the
Categorizer to evict the cached entry for that app_id. The categorizer exposes a
public invalidate(app_id) method that removes the matching key from the LRU
cache if present — a no-op for unknown app_ids.

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
migrations/). Users can create custom categories via the settings UI.

### App-First, Categories-Second

Tracking is always per-app; categories are derived metadata. The UsageRecord
type stores an app_id, a duration in seconds, and an optional category_id that
is populated lazily — the category is resolved at query time, not at write time.

### Policy Domain Enum

The data layer constructs one of these per active policy row — domain logic
never sees raw app_id/category_id columns:

- Policy::App — targets a specific app by app_id
- Policy::Category — targets all apps in a category_id

A policy always targets exactly one scope. The enum variants encode the target
so the domain evaluator can pattern-match without peeking at Option fields. If a
policy targets a category, it applies to all apps in that category. The
evaluator (evaluate() in policy/core/) receives a &[Policy] pre-filtered by the
app's app_id and resolved category_id s — the DB query does the join, not the
domain.

Tracking data is always stored per-app — categories are computed at query time
via the app_categories table.

## Browser Tab Detection

Browser windows are a single app_id containing multiple "logical apps" (tabs).
Detected via window title patterns:

| Browser        | Pattern                   | Notes                                     |
| -------------- | ------------------------- | ----------------------------------------- |
| Firefox        | ^(._) - ._ Firefox$       | Firefox uses em-dash (U+2014) globally    |
| Chromium       | ^(._) - ._ Google Chrome$ | Chromium uses regular hyphen, not em-dash |
| Chromium-based | ^(._) - ._ Chromium$      | Same hyphen separator                     |

Known locale sensitivity: Firefox's title format is "<page> — Mozilla Firefox"
in all locales (the app name is not localized). Chromium uses "<page> - Google
Chrome" (regular hyphen, not em-dash). These patterns are compiled statically
with Lazy<Regex>. Brave, Edge, Vivaldi, and Opera each have their own window
title format and are not matched — they appear as their base app_id (e.g.,
brave-browser). An app_categories entry is the preferred classification path for
less common browsers.

The captured group is the page title. URL detection is more invasive
(accessibility API), deferred until v2.

## Future: Machine Learning Classification

If manual categorization becomes a pain point, a local ML model (e.g., ort /
ONNX Runtime) could classify apps based on:

- app_id and window title patterns
- Usage time patterns (gaming = evening, work = morning)

The AiClassifier trait is designed for this — the v1 heuristic impl can be
swapped for an ONNX-based impl behind the same trait boundary.
