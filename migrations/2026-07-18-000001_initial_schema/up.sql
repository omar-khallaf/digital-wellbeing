-- Initial schema for Digital Wellbeing daemon
-- Forward-only, additive migrations

CREATE TABLE events (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    event_type  INTEGER NOT NULL CHECK(event_type IN (0, 1, 2, 3, 4, 5, 6, 7)),
    payload     TEXT NOT NULL,
    user_id     INTEGER NOT NULL,

    -- STORED generated columns: materialized from payload JSON at insert time
    timestamp   TEXT GENERATED ALWAYS AS (json_extract(payload, '$.t')) STORED NOT NULL,
    app_id      TEXT GENERATED ALWAYS AS (json_extract(payload, '$.a')) STORED,

    CHECK (
        (event_type = 0
            AND json_type(payload) IS 'object'
            AND json_type(payload, '$.t') IS 'text'
            AND json_type(payload, '$.a') IS 'text')
        OR
        (event_type IN (1, 2, 3, 4, 5, 6, 7)
            AND json_type(payload) IS 'object'
            AND json_type(payload, '$.t') IS 'text'
            AND json_extract(payload, '$.a') IS NULL)
    )
);

CREATE INDEX idx_events_ts ON events(timestamp);
CREATE INDEX idx_events_app_ts ON events(app_id, timestamp) WHERE app_id IS NOT NULL;
CREATE INDEX idx_events_user_id ON events(user_id, id);

CREATE TABLE daily_usage (
    date           TEXT NOT NULL,
    user_id        INTEGER NOT NULL,
    app_id         TEXT NOT NULL,
    total_seconds  INTEGER NOT NULL DEFAULT 0 CHECK(total_seconds >= 0),
    extended       INTEGER NOT NULL DEFAULT 0 CHECK(extended IN (0, 1)),
    updated_at     TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    PRIMARY KEY (date, user_id, app_id)
);

CREATE TABLE categories (
    id          INTEGER PRIMARY KEY,
    name        TEXT NOT NULL UNIQUE,
    color       TEXT,
    icon        TEXT,
    created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

CREATE TABLE policies (
    id          INTEGER PRIMARY KEY,
    name        TEXT NOT NULL,
    kind        INTEGER NOT NULL CHECK(kind IN (0, 1, 2)),
    category_id INTEGER REFERENCES categories(id) ON DELETE CASCADE,
    app_id      TEXT,
    created_by  INTEGER NOT NULL DEFAULT 0,
    owner_id    INTEGER NOT NULL DEFAULT 0,
    time_limit_seconds            INTEGER,
    extra_seconds                 INTEGER NOT NULL DEFAULT 600 CHECK(extra_seconds >= 0),
    notification_repeat_interval_seconds INTEGER,
    schedule_json                 TEXT NOT NULL DEFAULT '{}',
    active      INTEGER NOT NULL DEFAULT 1 CHECK(active IN (0, 1)),
    created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    updated_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),

    CHECK (
        (category_id IS NOT NULL AND app_id IS NULL)
        OR (category_id IS NULL AND app_id IS NOT NULL)
    ),
    CHECK (NOT (kind = 0 AND time_limit_seconds IS NOT NULL)),
    CHECK (NOT (kind IN (1, 2) AND time_limit_seconds IS NULL)),
    CHECK (time_limit_seconds IS NULL OR time_limit_seconds > 0),
    CHECK (notification_repeat_interval_seconds IS NULL
        OR (kind = 2 AND notification_repeat_interval_seconds > 0)),
    CHECK (json_type(schedule_json) IS 'object')
);

CREATE INDEX idx_policies_active ON policies(active) WHERE active = 1;
CREATE INDEX idx_policies_owner ON policies(owner_id);

-- Per-user app-to-category mappings. user_id=0 = system-global defaults.
-- Primary key changed from single-column (app_id) to composite (app_id, user_id)
-- during pre-deployment migration merge.
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

-- Seed built-in categories
INSERT INTO categories (name, color, icon) VALUES
    ('Productivity',    '#4CAF50', 'terminal'),
    ('Communication',   '#2196F3', 'chat'),
    ('Entertainment',   '#FF9800', 'games'),
    ('Social',          '#E91E63', 'globe'),
    ('Development',     '#9C27B0', 'code'),
    ('Utilities',       '#607D8B', 'settings'),
    ('Uncategorized',   '#9E9E9E', 'help');

-- Seed default app-to-category mappings (user_id=0 = system-global defaults)
INSERT OR IGNORE INTO app_categories (app_id, user_id, category_id, display_name) VALUES
    ('Alacritty',       0, (SELECT id FROM categories WHERE name='Productivity'),  'Alacritty'),
    ('kitty',           0, (SELECT id FROM categories WHERE name='Productivity'),  'Kitty'),
    ('foot',            0, (SELECT id FROM categories WHERE name='Productivity'),  'Foot'),
    ('wezterm',         0, (SELECT id FROM categories WHERE name='Productivity'),  'WezTerm'),
    ('gnome-terminal',  0, (SELECT id FROM categories WHERE name='Productivity'),  'Terminal'),
    ('konsole',         0, (SELECT id FROM categories WHERE name='Productivity'),  'Konsole'),
    ('terminator',      0, (SELECT id FROM categories WHERE name='Productivity'),  'Terminator'),
    ('tmux',            0, (SELECT id FROM categories WHERE name='Productivity'),  'tmux'),
    ('Code',            0, (SELECT id FROM categories WHERE name='Development'),   'VS Code'),
    ('code-oss',        0, (SELECT id FROM categories WHERE name='Development'),   'VS Code OSS'),
    ('zed',             0, (SELECT id FROM categories WHERE name='Development'),   'Zed'),
    ('jetbrains-idea',  0, (SELECT id FROM categories WHERE name='Development'),   'IntelliJ IDEA'),
    ('nvim',            0, (SELECT id FROM categories WHERE name='Development'),   'Neovim'),
    ('emacs',           0, (SELECT id FROM categories WHERE name='Development'),   'Emacs'),
    ('Atom',            0, (SELECT id FROM categories WHERE name='Development'),   'Atom'),
    ('Sublime_text',    0, (SELECT id FROM categories WHERE name='Development'),   'Sublime Text'),
    ('firefox',         0, (SELECT id FROM categories WHERE name='Social'),        'Firefox'),
    ('firefoxdeveloperedition', 0, (SELECT id FROM categories WHERE name='Social'), 'Firefox Dev'),
    ('Google-chrome',   0, (SELECT id FROM categories WHERE name='Social'),        'Google Chrome'),
    ('chromium-browser', 0, (SELECT id FROM categories WHERE name='Social'),       'Chromium'),
    ('brave-browser',   0, (SELECT id FROM categories WHERE name='Social'),        'Brave'),
    ('zen-browser',     0, (SELECT id FROM categories WHERE name='Social'),        'Zen Browser'),
    ('org.mozilla.firefox', 0, (SELECT id FROM categories WHERE name='Social'),    'Firefox (Flatpak)'),
    ('org.chromium.Chromium', 0, (SELECT id FROM categories WHERE name='Social'),  'Chromium (Flatpak)');
