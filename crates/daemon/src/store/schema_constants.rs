//! Table and column name constants — replaces diesel `table!` macros.
//!
//! Keep in sync with migration SQL files under `migrations/`.

// ── events ────────────────────────────────────────────────────────────────

pub mod events {
    pub const TABLE: &str = "events";
    pub const ID: &str = "id";
    pub const EVENT_TYPE: &str = "event_type";
    pub const USER_ID: &str = "user_id";
    pub const TIMESTAMP: &str = "timestamp";
    pub const APP_CLASS: &str = "app_class";
    pub const TITLE: &str = "title";
}

// ── apps ──────────────────────────────────────────────────────────────────

pub mod apps {
    pub const TABLE: &str = "apps";
    pub const ID: &str = "id";
    pub const APP_CLASS: &str = "app_class";
}

// ── daily_usage_by_app ─────────────────────────────────────────────────────

pub mod daily_usage_by_app {
    pub const TABLE: &str = "daily_usage_by_app";
    pub const DATE: &str = "date";
    pub const USER_ID: &str = "user_id";
    pub const APP_ID: &str = "app_id";
    pub const CLOSED_MILLIS: &str = "closed_millis";
    pub const OPEN_MILLIS: &str = "open_millis";
    pub const TOTAL_MILLIS: &str = "total_millis";
}

// ── daily_usage_by_category ────────────────────────────────────────────────

pub mod daily_usage_by_category {
    pub const TABLE: &str = "daily_usage_by_category";
    pub const DATE: &str = "date";
    pub const USER_ID: &str = "user_id";
    pub const CATEGORY: &str = "category";
    pub const CLOSED_MILLIS: &str = "closed_millis";
    pub const OPEN_MILLIS: &str = "open_millis";
    pub const TOTAL_MILLIS: &str = "total_millis";
}

// ── daily_usage_by_title ───────────────────────────────────────────────────

pub mod daily_usage_by_title {
    pub const TABLE: &str = "daily_usage_by_title";
    pub const DATE: &str = "date";
    pub const USER_ID: &str = "user_id";
    pub const APP_ID: &str = "app_id";
    pub const TITLE: &str = "title";
    pub const CLOSED_MILLIS: &str = "closed_millis";
    pub const OPEN_MILLIS: &str = "open_millis";
    pub const TOTAL_MILLIS: &str = "total_millis";
}

// ── policies ───────────────────────────────────────────────────────────────

pub mod policies {
    pub const TABLE: &str = "policies";
    pub const ID: &str = "id";
    pub const NAME: &str = "name";
    pub const PRIORITY: &str = "priority";
    pub const EFFECT: &str = "effect";
    pub const TARGET_TYPE: &str = "target_type";
    pub const APP_ID: &str = "app_id";
    pub const CATEGORY: &str = "category";
    pub const DOMAIN_PATTERN: &str = "domain_pattern";
    pub const TIME_LIMIT_MINUTES: &str = "time_limit_minutes";
    pub const USER_ID: &str = "user_id";
    pub const CREATED_BY: &str = "created_by";
}

// ── policy_schedules ───────────────────────────────────────────────────────

pub mod policy_schedules {
    pub const TABLE: &str = "policy_schedules";
    pub const POLICY_ID: &str = "policy_id";
    pub const START_MINUTE: &str = "start_minute";
    pub const END_MINUTE: &str = "end_minute";
    pub const DAY_MASK: &str = "day_mask";
}

// ── app_categories ─────────────────────────────────────────────────────────

pub mod app_categories {
    pub const TABLE: &str = "app_categories";
    pub const APP_ID: &str = "app_id";
    pub const USER_ID: &str = "user_id";
    pub const CATEGORY: &str = "category";
    pub const DISPLAY_NAME: &str = "display_name";
    pub const ICON_PATH: &str = "icon_path";
    pub const IGNORE: &str = "ignore";
    pub const UPDATED_AT: &str = "updated_at";
}

// ── __schema_migrations ─────────────────────────────────────────────

pub mod schema_migrations {
    pub const TABLE: &str = "__schema_migrations";
    pub const VERSION: &str = "version";
    pub const APPLIED_AT: &str = "applied_at";
}
