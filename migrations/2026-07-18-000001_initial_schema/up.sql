-- Initial schema for Digital Wellbeing daemon
-- Forward-only, additive migrations

CREATE TABLE events (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    event_type  INTEGER NOT NULL CHECK(event_type >= 0 AND event_type <= 7),
    user_id     INTEGER NOT NULL,
    timestamp   INTEGER NOT NULL,                -- epoch milliseconds (i64)
    app_id      TEXT,
    title       TEXT,

    -- Per-event-type shape enforcement:
    --   WindowFocused (0): requires app_id NOT NULL (identifies the app)
    --   Unfocused (1): app_id is optional — the interval's app is implied by
    --     the preceding WindowFocused event (derived by timeline builder and
    --     pre-buffer close resolver). Storing app_id here is redundant.
    --   Activity events (2: idle, 3: resumed): app_id identifies the focused app
    --   Power events (4-7: slept, shutdown, locked, loggedout):
    --     require app_id IS NULL AND title IS NULL
    CHECK (
        (event_type = 0 AND app_id IS NOT NULL)
        OR
        (event_type IN (1, 2, 3))
        OR
        (event_type >= 4 AND event_type <= 7 AND app_id IS NULL AND title IS NULL)
    ),
    CHECK (title IS NULL OR length(title) <= 1024)
);

CREATE INDEX idx_events_ts ON events(timestamp);
CREATE INDEX idx_events_app_ts ON events(app_id, timestamp) WHERE app_id IS NOT NULL;
CREATE INDEX idx_events_user_id ON events(user_id, id);

CREATE TABLE daily_usage (
    date           TEXT NOT NULL,
    user_id        INTEGER NOT NULL,
    app_id         TEXT NOT NULL,
    closed_millis  INTEGER NOT NULL DEFAULT 0 CHECK(closed_millis >= 0),
    open_millis    INTEGER NOT NULL DEFAULT 0 CHECK(open_millis >= 0),
    extended       INTEGER NOT NULL DEFAULT 0 CHECK(extended IN (0, 1)),
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
    action      INTEGER NOT NULL CHECK(action IN (0, 1, 2)),
    category_id INTEGER REFERENCES categories(id) ON DELETE CASCADE,
    app_id      TEXT,
    created_by  INTEGER NOT NULL DEFAULT 0,
    owner_id    INTEGER NOT NULL DEFAULT 0,
    time_limit_minutes            INTEGER,
    extra_minutes                 INTEGER NOT NULL DEFAULT 10 CHECK(extra_minutes >= 0),
    notification_repeat_interval_minutes INTEGER,
    schedule_start_hour           INTEGER CHECK (schedule_start_hour BETWEEN 0 AND 23),
    schedule_end_hour             INTEGER CHECK (schedule_end_hour BETWEEN 0 AND 23),
    schedule_days                 TEXT NOT NULL DEFAULT '[]',
    active                        INTEGER NOT NULL DEFAULT 1 CHECK(active IN (0, 1)),
    created_at                    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    updated_at                    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),

    CHECK (
        (category_id IS NOT NULL AND app_id IS NULL)
        OR (category_id IS NULL AND app_id IS NOT NULL)
    ),
    CHECK (NOT (action = 0 AND time_limit_minutes IS NOT NULL)),
    CHECK (NOT (action IN (1, 2) AND time_limit_minutes IS NULL)),
    CHECK (time_limit_minutes IS NULL OR time_limit_minutes > 0),
    CHECK (notification_repeat_interval_minutes IS NULL
        OR (action = 2 AND notification_repeat_interval_minutes > 0)),
    CHECK (NOT (schedule_start_hour IS NOT NULL AND schedule_end_hour IS NULL)
          AND NOT (schedule_start_hour IS NULL AND schedule_end_hour IS NOT NULL)),
    CHECK (
        schedule_days IS NULL
        OR json_type(schedule_days) IS 'array'
    )
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
