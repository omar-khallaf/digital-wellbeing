//! Diesel table definitions — manually maintained to match migration SQL.
//!
//! WARNING: This file is the single source of truth for compile-time
//! table access. Keep in sync with migration files.

diesel::table! {
    events (id) {
        id -> Integer,
        event_type -> Integer,
        user_id -> Integer,
        timestamp -> BigInt,
        app_class -> Nullable<Text>,
        title -> Nullable<Text>,
    }
}

diesel::table! {
    apps (id) {
        id -> Integer,
        app_class -> Text,
    }
}

diesel::table! {
    daily_usage_by_app (date, user_id, app_id) {
        date -> Text,
        user_id -> Integer,
        app_id -> Integer,
        closed_millis -> BigInt,
        open_millis -> BigInt,
        total_millis -> BigInt,
    }
}

diesel::table! {
    daily_usage_by_category (date, user_id, category_id) {
        date -> Text,
        user_id -> Integer,
        category_id -> Integer,
        closed_millis -> BigInt,
        open_millis -> BigInt,
        total_millis -> BigInt,
    }
}

diesel::table! {
    daily_usage_by_title (date, user_id, app_id, title) {
        date -> Text,
        user_id -> Integer,
        app_id -> Integer,
        title -> Text,
        closed_millis -> BigInt,
        open_millis -> BigInt,
        total_millis -> BigInt,
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
        priority -> Integer,
        effect -> Integer,
        target_type -> Integer,
        app_id -> Nullable<Integer>,
        category_id -> Nullable<Integer>,
        domain_pattern -> Nullable<Text>,
        time_limit_minutes -> Nullable<Integer>,
        user_id -> Integer,
        created_by -> Integer,
        created_at -> Text,
        updated_at -> Text,
    }
}

diesel::table! {
    policy_schedules (policy_id, start_minute, end_minute) {
        policy_id -> Integer,
        start_minute -> Integer,
        end_minute -> Integer,
        day_mask -> Integer,
    }
}

diesel::table! {
    app_categories (app_id, user_id) {
        app_id -> Integer,
        user_id -> Integer,
        category_id -> Nullable<Integer>,
        display_name -> Nullable<Text>,
        icon_path -> Nullable<Text>,
        ignore -> Bool,
        updated_at -> Text,
    }
}

diesel::joinable!(daily_usage_by_app -> apps (app_id));
diesel::joinable!(daily_usage_by_title -> apps (app_id));
diesel::joinable!(daily_usage_by_category -> categories (category_id));
diesel::joinable!(policy_schedules -> policies (policy_id));
diesel::joinable!(policies -> apps (app_id));
diesel::joinable!(policies -> categories (category_id));
diesel::joinable!(app_categories -> apps (app_id));

diesel::allow_columns_to_appear_in_same_group_by_clause!(
    apps::app_class,
    daily_usage_by_title::title,
);

diesel::allow_tables_to_appear_in_same_query!(
    events,
    apps,
    daily_usage_by_app,
    daily_usage_by_category,
    daily_usage_by_title,
    categories,
    policies,
    policy_schedules,
    app_categories,
);
