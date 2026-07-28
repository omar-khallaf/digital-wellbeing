-- Initial schema for Digital Wellbeing daemon
-- Forward-only, additive migrations

CREATE TABLE events (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    event_type  INTEGER NOT NULL CHECK(event_type >= 0 AND event_type <= 8),
    user_id     INTEGER NOT NULL,
    timestamp   INTEGER NOT NULL,
    app_class   TEXT,
    title       TEXT CHECK(length(title) <= 1024),

    CHECK (
        (event_type IN (0, 8) AND app_class IS NOT NULL AND title IS NOT NULL)
        OR (event_type BETWEEN 1 AND 7 AND app_class IS NULL AND title IS NULL)
    )
);

CREATE INDEX idx_events_ts ON events(timestamp);
CREATE INDEX idx_events_app_ts ON events(app_class, timestamp) WHERE app_class IS NOT NULL;
CREATE INDEX idx_events_user_id ON events(user_id, id);

-- ── `apps` registry ───────────────────────────────────────────────────────────

CREATE TABLE apps (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    app_class  TEXT NOT NULL UNIQUE CHECK(length(app_class) > 0)
);

-- ── `daily_usage_by_app` (replaces old `daily_usage`) ─────────────────────────

CREATE TABLE daily_usage_by_app (
    date          TEXT NOT NULL,
    user_id       INTEGER NOT NULL,
    app_id        INTEGER NOT NULL REFERENCES apps(id),
    closed_millis INTEGER NOT NULL DEFAULT 0 CHECK(closed_millis >= 0),
    open_millis   INTEGER NOT NULL DEFAULT 0 CHECK(open_millis >= 0),
    total_millis  INTEGER GENERATED ALWAYS AS (closed_millis + open_millis) VIRTUAL,
    PRIMARY KEY (date, user_id, app_id)
);

CREATE INDEX idx_daily_usage_by_app_date ON daily_usage_by_app(date);
CREATE INDEX idx_daily_usage_by_app_user_date ON daily_usage_by_app(user_id, date);

CREATE TABLE daily_usage_by_title (
    date          TEXT NOT NULL,
    user_id       INTEGER NOT NULL,
    app_id        INTEGER NOT NULL REFERENCES apps(id),
    title         TEXT NOT NULL CHECK(length(title) <= 1024),
    closed_millis INTEGER NOT NULL DEFAULT 0 CHECK(closed_millis >= 0),
    open_millis   INTEGER NOT NULL DEFAULT 0 CHECK(open_millis >= 0),
    total_millis  INTEGER GENERATED ALWAYS AS (closed_millis + open_millis) VIRTUAL,
    PRIMARY KEY (date, user_id, app_id, title)
);

CREATE INDEX idx_daily_usage_by_title_date ON daily_usage_by_title(date);
CREATE INDEX idx_daily_usage_by_title_user_date ON daily_usage_by_title(user_id, date);

-- ── `daily_usage_by_category` ─────────────────────────────────────────────────

CREATE TABLE daily_usage_by_category (
    date           TEXT NOT NULL,
    user_id        INTEGER NOT NULL,
    category_id    INTEGER NOT NULL REFERENCES categories(id),
    closed_millis  INTEGER NOT NULL DEFAULT 0 CHECK(closed_millis >= 0),
    open_millis    INTEGER NOT NULL DEFAULT 0 CHECK(open_millis >= 0),
    total_millis   INTEGER GENERATED ALWAYS AS (closed_millis + open_millis) VIRTUAL,
    PRIMARY KEY (date, user_id, category_id)
);

CREATE INDEX idx_daily_usage_by_category_date ON daily_usage_by_category(date);
CREATE INDEX idx_daily_usage_by_category_user_date ON daily_usage_by_category(user_id, date);

CREATE TABLE categories (
    id          INTEGER PRIMARY KEY,
    name        TEXT NOT NULL UNIQUE,
    color       TEXT,
    icon        TEXT,
    created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

-- ── New `policies` table ──────────────────────────────────────────────────────
-- effect: 0=Allow, 1=Block, 2=TimeLimit, 3=Notify
-- target_type: 0=App, 1=Category, 2=Domain, 3=Any

CREATE TABLE policies (
    id                 INTEGER PRIMARY KEY,
    name               TEXT NOT NULL CHECK(length(name) > 0),
    priority           INTEGER NOT NULL DEFAULT 100 CHECK(priority >= 0),
    effect             INTEGER NOT NULL CHECK(effect IN (0,1,2,3)),
    target_type        INTEGER NOT NULL DEFAULT 3 CHECK(target_type IN (0,1,2,3)),
    app_id             INTEGER REFERENCES apps(id),
    category_id        INTEGER REFERENCES categories(id),
    domain_pattern     TEXT,
    time_limit_minutes INTEGER,
    user_id            INTEGER NOT NULL,
    created_by         INTEGER NOT NULL DEFAULT 0,
    created_at         TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    updated_at         TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),

    -- Exactly one target type must be set (the other two NULL).
    CHECK (
        (target_type = 0 AND app_id IS NOT NULL AND category_id IS NULL AND domain_pattern IS NULL)
        OR (target_type = 1 AND app_id IS NULL AND category_id IS NOT NULL AND domain_pattern IS NULL)
        OR (target_type = 2 AND app_id IS NULL AND category_id IS NULL AND domain_pattern IS NOT NULL)
        OR (target_type = 3 AND app_id IS NULL AND category_id IS NULL AND domain_pattern IS NULL)
    ),
    -- TimeLimit(2) and Notify(3) require time_limit_minutes.
    CHECK (effect NOT IN (2,3) OR (time_limit_minutes IS NOT NULL AND time_limit_minutes > 0)),
    -- Allow(0) and Block(1) must NOT have time_limit_minutes.
    CHECK (effect NOT IN (0,1) OR time_limit_minutes IS NULL)
);

CREATE INDEX idx_policies_user_id ON policies(user_id);
CREATE INDEX idx_policies_priority ON policies(priority);

-- ── `policy_schedules` — normalized, one row per time window ──────────────────

CREATE TABLE policy_schedules (
    policy_id    INTEGER NOT NULL REFERENCES policies(id) ON DELETE CASCADE,
    start_minute INTEGER NOT NULL CHECK(start_minute BETWEEN 0 AND 1439),
    end_minute   INTEGER NOT NULL CHECK(end_minute BETWEEN 0 AND 1439),
    day_mask     INTEGER NOT NULL DEFAULT 0 CHECK(day_mask BETWEEN 0 AND 127),
    CHECK(start_minute != end_minute)
);

CREATE INDEX idx_policy_schedules_policy_id ON policy_schedules(policy_id);

-- Per-user app-to-category mappings. user_id=0 = system-global defaults.
CREATE TABLE app_categories (
    app_id         INTEGER NOT NULL REFERENCES apps(id) ON DELETE CASCADE,
    user_id        INTEGER NOT NULL DEFAULT 0,
    category_id    INTEGER REFERENCES categories(id) ON DELETE CASCADE,
    display_name   TEXT,
    icon_path      TEXT,
    ignore         INTEGER NOT NULL DEFAULT 0 CHECK(ignore IN (0, 1)),
    updated_at     TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    PRIMARY KEY (app_id, user_id)
);

-- Seed built-in categories
INSERT OR IGNORE INTO categories (name, color, icon) VALUES
    ('Productivity',    '#4CAF50', 'terminal'),
    ('Communication',   '#2196F3', 'chat'),
    ('Entertainment',   '#FF9800', 'games'),
    ('Social',          '#E91E63', 'globe'),
    ('Development',     '#9C27B0', 'code'),
    ('Utilities',       '#607D8B', 'settings'),
    ('Uncategorized',   '#9E9E9E', 'help');

-- Seed apps so app_categories can reference them by integer FK.
INSERT OR IGNORE INTO apps (app_class) VALUES
    ('Alacritty'), ('kitty'), ('wezterm'), ('gnome-terminal'),
    ('konsole'), ('terminator'), ('Code'), ('code-oss'), ('zed'),
    ('jetbrains-idea'), ('Atom'), ('Sublime_text'), ('firefox'),
    ('Google-chrome'), ('chromium-browser'), ('brave-browser'),
    ('zen-browser'), ('org.mozilla.firefox'), ('org.chromium.Chromium');

-- Seed default app-to-category mappings (user_id=0 = system-global defaults)
INSERT OR IGNORE INTO app_categories (app_id, user_id, category_id, display_name) VALUES
    ((SELECT id FROM apps WHERE app_class='Alacritty'),        0, (SELECT id FROM categories WHERE name='Productivity'),  'Alacritty'),
    ((SELECT id FROM apps WHERE app_class='kitty'),            0, (SELECT id FROM categories WHERE name='Productivity'),  'Kitty'),
    ((SELECT id FROM apps WHERE app_class='wezterm'),          0, (SELECT id FROM categories WHERE name='Productivity'),  'WezTerm'),
    ((SELECT id FROM apps WHERE app_class='gnome-terminal'),   0, (SELECT id FROM categories WHERE name='Productivity'),  'Terminal'),
    ((SELECT id FROM apps WHERE app_class='konsole'),          0, (SELECT id FROM categories WHERE name='Productivity'),  'Konsole'),
    ((SELECT id FROM apps WHERE app_class='terminator'),       0, (SELECT id FROM categories WHERE name='Productivity'),  'Terminator'),
    ((SELECT id FROM apps WHERE app_class='Code'),             0, (SELECT id FROM categories WHERE name='Development'),   'VS Code'),
    ((SELECT id FROM apps WHERE app_class='code-oss'),         0, (SELECT id FROM categories WHERE name='Development'),   'VS Code OSS'),
    ((SELECT id FROM apps WHERE app_class='zed'),              0, (SELECT id FROM categories WHERE name='Development'),   'Zed'),
    ((SELECT id FROM apps WHERE app_class='jetbrains-idea'),   0, (SELECT id FROM categories WHERE name='Development'),   'IntelliJ IDEA'),
    ((SELECT id FROM apps WHERE app_class='Atom'),             0, (SELECT id FROM categories WHERE name='Development'),   'Atom'),
    ((SELECT id FROM apps WHERE app_class='Sublime_text'),     0, (SELECT id FROM categories WHERE name='Development'),   'Sublime Text'),
    ((SELECT id FROM apps WHERE app_class='firefox'),          0, (SELECT id FROM categories WHERE name='Social'),        'Firefox'),
    ((SELECT id FROM apps WHERE app_class='Google-chrome'),    0, (SELECT id FROM categories WHERE name='Social'),        'Google Chrome'),
    ((SELECT id FROM apps WHERE app_class='chromium-browser'), 0, (SELECT id FROM categories WHERE name='Social'),        'Chromium'),
    ((SELECT id FROM apps WHERE app_class='brave-browser'),    0, (SELECT id FROM categories WHERE name='Social'),        'Brave'),
    ((SELECT id FROM apps WHERE app_class='zen-browser'),      0, (SELECT id FROM categories WHERE name='Social'),        'Zen Browser'),
    ((SELECT id FROM apps WHERE app_class='org.mozilla.firefox'),   0, (SELECT id FROM categories WHERE name='Social'),   'Firefox (Flatpak)'),
    ((SELECT id FROM apps WHERE app_class='org.chromium.Chromium'), 0, (SELECT id FROM categories WHERE name='Social'),   'Chromium (Flatpak)');
