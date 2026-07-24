//! Diesel table definitions matching the migration SQL.
//! Generated manually to match the initial migration schema.

diesel::table! {
    events (id) {
        id -> Integer,
        event_type -> Integer,
        user_id -> Integer,
        timestamp -> BigInt,
        app_id -> Nullable<Text>,
        title -> Nullable<Text>,
    }
}

diesel::table! {
    daily_usage (date, user_id, app_id) {
        date -> Text,
        user_id -> Integer,
        app_id -> Text,
        closed_millis -> Integer,
        open_millis -> Integer,
        extended -> Bool,
    }
}

diesel::table! {
    categories (id) {
        id -> Integer,
        name -> Text,
        color -> Nullable<Text>,
        icon -> Nullable<Text>,
        created_at -> Text,
    }
}

diesel::table! {
    policies (id) {
        id -> Integer,
        name -> Text,
        action -> Integer,
        category_id -> Nullable<Integer>,
        app_id -> Nullable<Text>,
        created_by -> Integer,
        owner_id -> Integer,
        time_limit_minutes -> Nullable<Integer>,
        extra_minutes -> Integer,
        notification_repeat_interval_minutes -> Nullable<Integer>,
        schedule_start_hour -> Nullable<Integer>,
        schedule_end_hour -> Nullable<Integer>,
        schedule_days -> Text,
        active -> Bool,
        created_at -> Text,
        updated_at -> Text,
    }
}

diesel::table! {
    app_categories (app_id, user_id) {
        app_id -> Text,
        user_id -> Integer,
        category_id -> Nullable<Integer>,
        display_name -> Nullable<Text>,
        icon_path -> Nullable<Text>,
        ignore -> Bool,
        updated_at -> Text,
    }
}
