use chrono::{DateTime, Datelike, Timelike, Utc};
use diesel::BoolExpressionMethods;
use diesel::ExpressionMethods;
use diesel::OptionalExtension;
use diesel::QueryDsl;
use diesel_async::RunQueryDsl;
use serde::Deserialize;
use wellbeing_core::Clock;
use wellbeing_core::*;

use crate::store::connection::DbConn;
use crate::store::schema::{app_categories, daily_usage, policies};

pub enum TimeLimitedApp {
    Normal(i64, i64),
    Extended(i64, i64),
}

impl TimeLimitedApp {
    pub fn remaining(&self) -> i64 {
        match self {
            Self::Normal(used, limit) | Self::Extended(used, limit) => limit - used,
        }
    }

    pub fn can_extend(&self) -> bool {
        matches!(self, Self::Normal(..))
    }

    pub fn effective_limit(&self) -> i64 {
        match self {
            Self::Normal(_, limit) | Self::Extended(_, limit) => *limit,
        }
    }
}

pub struct TimeTrackedApp {
    pub used: i64,
    pub limit: i64,
}

impl TimeTrackedApp {
    pub fn remaining(&self) -> i64 {
        (self.limit - self.used).max(0)
    }

    pub fn is_exceeded(&self) -> bool {
        self.used >= self.limit
    }
}

pub enum TrackedApp {
    TimeLimited(TimeLimitedApp),
    TimeTracked(TimeTrackedApp),
}

#[must_use]
pub enum PolicyVerdict {
    Ok,
    Block {
        policy_id: PolicyId,
        reason: BlockReason,
        remaining: i64,
    },
    Notify {
        policy_id: PolicyId,
        repeat_interval: Option<i64>,
    },
}

#[derive(Debug, Clone)]
pub enum PolicyConfig {
    Block {
        id: PolicyId,
        app_id: Option<AppId>,
        category_id: Option<CategoryId>,
        active: bool,
    },
    TimeLimit {
        id: PolicyId,
        app_id: Option<AppId>,
        category_id: Option<CategoryId>,
        time_limit_seconds: i64,
        extra_seconds: i64,
        active: bool,
    },
    Notify {
        id: PolicyId,
        app_id: Option<AppId>,
        category_id: Option<CategoryId>,
        time_limit_seconds: i64,
        notification_repeat_interval_seconds: Option<i64>,
        active: bool,
    },
}

impl PolicyConfig {
    pub fn id(&self) -> PolicyId {
        match self {
            Self::Block { id, .. } | Self::TimeLimit { id, .. } | Self::Notify { id, .. } => *id,
        }
    }

    pub fn app_id(&self) -> Option<&AppId> {
        match self {
            Self::Block { app_id, .. }
            | Self::TimeLimit { app_id, .. }
            | Self::Notify { app_id, .. } => app_id.as_ref(),
        }
    }

    pub fn category_id(&self) -> Option<CategoryId> {
        match self {
            Self::Block { category_id, .. }
            | Self::TimeLimit { category_id, .. }
            | Self::Notify { category_id, .. } => *category_id,
        }
    }

    pub fn active(&self) -> bool {
        match self {
            Self::Block { active, .. }
            | Self::TimeLimit { active, .. }
            | Self::Notify { active, .. } => *active,
        }
    }
}

pub struct TimeWindow {
    pub start_hour: u8,
    pub end_hour: u8,
    pub days: Vec<u8>,
}

impl TimeWindow {
    pub fn from_json(json: &str) -> serde_json::Result<Vec<TimeWindow>> {
        #[derive(Deserialize)]
        struct ScheduleJson {
            time_windows: Option<Vec<WindowDef>>,
        }

        #[derive(Deserialize)]
        struct WindowDef {
            start_hour: u8,
            end_hour: u8,
            #[serde(default)]
            days: Option<Vec<u8>>,
        }

        if json.is_empty() {
            return Ok(Vec::new());
        }

        let schedule: ScheduleJson = serde_json::from_str(json)?;
        Ok(schedule
            .time_windows
            .unwrap_or_default()
            .into_iter()
            .map(|w| TimeWindow {
                start_hour: w.start_hour.min(23),
                end_hour: w.end_hour.min(23),
                days: w.days.unwrap_or_default(),
            })
            .collect())
    }

    pub fn is_active(&self, now: DateTime<Utc>) -> bool {
        if !self.days.is_empty() {
            let day_num = now.weekday().num_days_from_sunday() as u8;
            if !self.days.contains(&day_num) {
                return false;
            }
        }

        let hour = now.hour() as u8;
        if self.start_hour <= self.end_hour {
            hour >= self.start_hour && hour < self.end_hour
        } else {
            hour >= self.start_hour || hour < self.end_hour
        }
    }
}

pub fn app_state(usage: (i64, bool), policy: &PolicyConfig) -> TrackedApp {
    match policy {
        PolicyConfig::Block { .. } => {
            unreachable!("Block policy has no tracked state")
        }
        PolicyConfig::TimeLimit {
            time_limit_seconds,
            extra_seconds,
            ..
        } => {
            let app = if usage.1 {
                TimeLimitedApp::Extended(usage.0, time_limit_seconds + extra_seconds)
            } else {
                TimeLimitedApp::Normal(usage.0, *time_limit_seconds)
            };
            TrackedApp::TimeLimited(app)
        }
        PolicyConfig::Notify {
            time_limit_seconds, ..
        } => TrackedApp::TimeTracked(TimeTrackedApp {
            used: usage.0,
            limit: *time_limit_seconds,
        }),
    }
}

fn eval_block_policy(policy: &PolicyConfig, first_block: bool) -> Option<PolicyVerdict> {
    if first_block {
        return None;
    }
    match policy {
        PolicyConfig::Block { app_id, id, .. } => {
            let reason = if app_id.is_some() {
                BlockReason::AppBlock
            } else {
                BlockReason::CategoryBlock
            };
            Some(PolicyVerdict::Block {
                policy_id: *id,
                reason,
                remaining: 0,
            })
        }
        _ => None,
    }
}

fn eval_timelimit_policy(
    policy: &PolicyConfig,
    elapsed_usage: i64,
    extended: bool,
    first_block: bool,
) -> Option<PolicyVerdict> {
    if first_block {
        return None;
    }
    match policy {
        PolicyConfig::TimeLimit {
            id,
            app_id,
            time_limit_seconds,
            extra_seconds,
            ..
        } => {
            let effective_limit = if extended {
                time_limit_seconds + extra_seconds
            } else {
                *time_limit_seconds
            };
            let remaining = effective_limit - elapsed_usage;
            if remaining <= 0 {
                let reason = if app_id.is_some() {
                    BlockReason::AppTimeLimit
                } else {
                    BlockReason::CategoryTimeLimit
                };
                Some(PolicyVerdict::Block {
                    policy_id: *id,
                    reason,
                    remaining,
                })
            } else {
                None
            }
        }
        _ => None,
    }
}

fn eval_notify_policy(
    policy: &PolicyConfig,
    elapsed_usage: i64,
    first_notify: bool,
) -> Option<PolicyVerdict> {
    if first_notify {
        return None;
    }
    match policy {
        PolicyConfig::Notify {
            id,
            time_limit_seconds,
            notification_repeat_interval_seconds,
            ..
        } => {
            if elapsed_usage >= *time_limit_seconds {
                Some(PolicyVerdict::Notify {
                    policy_id: *id,
                    repeat_interval: *notification_repeat_interval_seconds,
                })
            } else {
                None
            }
        }
        _ => None,
    }
}

pub fn evaluate(
    _app_id: &AppId,
    policies: &[PolicyConfig],
    elapsed_usage: i64,
    extended: bool,
) -> PolicyVerdict {
    let mut first_block: Option<PolicyVerdict> = None;
    let mut first_notify: Option<PolicyVerdict> = None;

    for policy in policies {
        if !policy.active() {
            continue;
        }
        if let Some(block) = eval_block_policy(policy, first_block.is_some()).or_else(|| {
            eval_timelimit_policy(policy, elapsed_usage, extended, first_block.is_some())
        }) {
            first_block = Some(block);
        } else if let Some(notify) =
            eval_notify_policy(policy, elapsed_usage, first_notify.is_some())
        {
            first_notify = Some(notify);
        }
    }
    first_block.or(first_notify).unwrap_or(PolicyVerdict::Ok)
}

#[derive(Debug, Clone, diesel::Queryable)]
pub(crate) struct PolicyRow {
    id: i32,
    name: String,
    kind: i32,
    category_id: Option<i32>,
    app_id: Option<String>,
    created_by: i32,
    owner_id: i32,
    time_limit_seconds: Option<i32>,
    extra_seconds: i32,
    notification_repeat_interval_seconds: Option<i32>,
    schedule_json: String,
    active: bool,
    created_at: String,
    updated_at: String,
}

impl PolicyRow {
    pub(crate) fn into_domain_policy(self) -> wellbeing_core::Policy {
        wellbeing_core::Policy {
            id: PolicyId(self.id as i64),
            name: self.name,
            kind: match self.kind {
                0 => PolicyKind::Block,
                1 => PolicyKind::TimeLimit,
                2 => PolicyKind::Notify,
                _ => PolicyKind::Block,
            },
            app_id: self.app_id.unwrap_or_default(),
            category_id: self.category_id.unwrap_or(0) as i64,
            time_limit_seconds: self.time_limit_seconds.unwrap_or(0) as i64,
            extra_seconds: self.extra_seconds as i64,
            notification_repeat_interval_seconds: self
                .notification_repeat_interval_seconds
                .unwrap_or(0) as i64,
            schedule_json: self.schedule_json,
            active: self.active,
            created_by: self.created_by as u32,
            owner_id: self.owner_id as u32,
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }
}

pub async fn resolve_policies_for_app(
    conn: &mut DbConn,
    app_id: &AppId,
    categories: &[CategoryId],
    uid: Uid,
) -> anyhow::Result<Vec<wellbeing_core::Policy>> {
    let cat_ids: Vec<i32> = categories.iter().map(|c| c.0 as i32).collect();

    let rows: Vec<PolicyRow> = if cat_ids.is_empty() {
        policies::table
            .filter(policies::active.eq(true))
            .filter(policies::owner_id.eq(uid.0 as i32))
            .filter(policies::app_id.eq(app_id.as_str()))
            .load(conn)
            .await?
    } else {
        policies::table
            .filter(policies::active.eq(true))
            .filter(policies::owner_id.eq(uid.0 as i32))
            .filter(
                policies::app_id
                    .eq(app_id.as_str())
                    .or(policies::category_id.eq_any(cat_ids)),
            )
            .load(conn)
            .await?
    };

    Ok(rows.into_iter().map(|r| r.into_domain_policy()).collect())
}

pub async fn resolve_categories_for_app(
    conn: &mut DbConn,
    app_id: &AppId,
    uid: Uid,
) -> anyhow::Result<Vec<CategoryId>> {
    let rows: Vec<Option<i32>> = app_categories::table
        .filter(app_categories::app_id.eq(app_id.as_str()))
        .filter(app_categories::category_id.is_not_null())
        .filter(app_categories::ignore.eq(false))
        .filter(app_categories::user_id.eq(uid.0 as i32))
        .select(app_categories::category_id)
        .load(conn)
        .await?;

    if !rows.is_empty() {
        return Ok(rows
            .into_iter()
            .flatten()
            .map(|id| CategoryId(id as i64))
            .collect());
    }

    let fallback: Vec<Option<i32>> = app_categories::table
        .filter(app_categories::app_id.eq(app_id.as_str()))
        .filter(app_categories::category_id.is_not_null())
        .filter(app_categories::ignore.eq(false))
        .filter(app_categories::user_id.eq(0i32))
        .select(app_categories::category_id)
        .load(conn)
        .await?;

    Ok(fallback
        .into_iter()
        .flatten()
        .map(|id| CategoryId(id as i64))
        .collect())
}

pub async fn get_daily_usage_for_app(
    conn: &mut DbConn,
    app_id: &AppId,
    uid: Uid,
    clock: &dyn Clock,
) -> anyhow::Result<Option<(i64, bool)>> {
    let today = clock.now().format("%Y-%m-%d").to_string();

    let result: Option<(i32, bool)> = daily_usage::table
        .filter(daily_usage::user_id.eq(uid.0 as i32))
        .filter(daily_usage::app_id.eq(app_id.as_str()))
        .filter(daily_usage::date.eq(&today))
        .select((daily_usage::total_seconds, daily_usage::extended))
        .first(conn)
        .await
        .optional()?;

    Ok(result.map(|(secs, ext)| (secs as i64, ext)))
}

impl From<wellbeing_core::Policy> for PolicyConfig {
    fn from(p: wellbeing_core::Policy) -> Self {
        let app_id = if p.app_id.is_empty() {
            None
        } else {
            AppId::new(&p.app_id).ok()
        };
        let category_id = if p.category_id == 0 {
            None
        } else {
            Some(CategoryId(p.category_id))
        };

        match p.kind {
            PolicyKind::Block => PolicyConfig::Block {
                id: p.id,
                app_id,
                category_id,
                active: p.active,
            },
            PolicyKind::TimeLimit => PolicyConfig::TimeLimit {
                id: p.id,
                app_id,
                category_id,
                time_limit_seconds: p.time_limit_seconds.max(1),
                extra_seconds: p.extra_seconds,
                active: p.active,
            },
            PolicyKind::Notify => PolicyConfig::Notify {
                id: p.id,
                app_id,
                category_id,
                time_limit_seconds: p.time_limit_seconds.max(1),
                notification_repeat_interval_seconds: if p.notification_repeat_interval_seconds == 0
                {
                    None
                } else {
                    Some(p.notification_repeat_interval_seconds)
                },
                active: p.active,
            },
        }
    }
}

pub fn filter_policies_by_schedule(
    policies: Vec<wellbeing_core::Policy>,
    now: DateTime<Utc>,
) -> Vec<PolicyConfig> {
    policies
        .into_iter()
        .filter(|p| {
            if p.schedule_json.is_empty() {
                return true;
            }
            TimeWindow::from_json(&p.schedule_json)
                .map(|windows| windows.is_empty() || windows.iter().any(|w| w.is_active(now)))
                .unwrap_or(false)
        })
        .map(PolicyConfig::from)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_policy(id: i64, kind: PolicyKind, app: bool, limit: Option<i64>) -> PolicyConfig {
        let app_id = if app {
            Some(AppId::new("test.app").unwrap())
        } else {
            None
        };
        let category_id = if app { None } else { Some(CategoryId(1)) };
        match kind {
            PolicyKind::Block => PolicyConfig::Block {
                id: PolicyId(id),
                app_id,
                category_id,
                active: true,
            },
            PolicyKind::TimeLimit => PolicyConfig::TimeLimit {
                id: PolicyId(id),
                app_id,
                category_id,
                time_limit_seconds: limit.unwrap_or(3600),
                extra_seconds: 300,
                active: true,
            },
            PolicyKind::Notify => PolicyConfig::Notify {
                id: PolicyId(id),
                app_id,
                category_id,
                time_limit_seconds: limit.unwrap_or(3600),
                notification_repeat_interval_seconds: None,
                active: true,
            },
        }
    }

    fn make_policy_full(
        id: i64,
        kind: PolicyKind,
        app: bool,
        limit: Option<i64>,
        extra: i64,
        repeat: Option<i64>,
        active: bool,
    ) -> PolicyConfig {
        let app_id = if app {
            Some(AppId::new("test.app").unwrap())
        } else {
            None
        };
        let category_id = if app { None } else { Some(CategoryId(1)) };
        match kind {
            PolicyKind::Block => PolicyConfig::Block {
                id: PolicyId(id),
                app_id,
                category_id,
                active,
            },
            PolicyKind::TimeLimit => PolicyConfig::TimeLimit {
                id: PolicyId(id),
                app_id,
                category_id,
                time_limit_seconds: limit.unwrap_or(3600),
                extra_seconds: extra,
                active,
            },
            PolicyKind::Notify => PolicyConfig::Notify {
                id: PolicyId(id),
                app_id,
                category_id,
                time_limit_seconds: limit.unwrap_or(3600),
                notification_repeat_interval_seconds: repeat,
                active,
            },
        }
    }

    #[test]
    fn test_time_limited_normal_remaining() {
        let app = TimeLimitedApp::Normal(100, 3600);
        assert_eq!(app.remaining(), 3500);
        assert!(app.can_extend());
        assert_eq!(app.effective_limit(), 3600);
    }

    #[test]
    fn test_time_limited_extended_remaining() {
        let app = TimeLimitedApp::Extended(4000, 5400);
        assert_eq!(app.remaining(), 1400);
        assert!(!app.can_extend());
        assert_eq!(app.effective_limit(), 5400);
    }

    #[test]
    fn test_time_limited_exceeded() {
        let app = TimeLimitedApp::Normal(4000, 3600);
        assert_eq!(app.remaining(), -400);
        assert!(app.can_extend());
    }

    #[test]
    fn test_time_tracked_remaining_within_limit() {
        let app = TimeTrackedApp {
            used: 100,
            limit: 3600,
        };
        assert_eq!(app.remaining(), 3500);
        assert!(!app.is_exceeded());
    }

    #[test]
    fn test_time_tracked_exceeded() {
        let app = TimeTrackedApp {
            used: 4000,
            limit: 3600,
        };
        assert_eq!(app.remaining(), 0);
        assert!(app.is_exceeded());
    }

    #[test]
    fn test_time_tracked_at_limit() {
        let app = TimeTrackedApp {
            used: 3600,
            limit: 3600,
        };
        assert_eq!(app.remaining(), 0);
        assert!(app.is_exceeded());
    }

    #[test]
    fn test_time_window_empty_json_returns_empty() {
        let windows = TimeWindow::from_json("").unwrap();
        assert!(windows.is_empty());
    }

    #[test]
    fn test_time_window_single_no_days() {
        let json = r#"{"time_windows": [{"start_hour": 9, "end_hour": 17}]}"#;
        let windows = TimeWindow::from_json(json).unwrap();
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].start_hour, 9);
        assert_eq!(windows[0].end_hour, 17);
        assert!(windows[0].days.is_empty());
    }

    #[test]
    fn test_time_window_with_days() {
        let json = r#"{"time_windows": [{"start_hour": 9, "end_hour": 17, "days": [1,2,3,4,5]}]}"#;
        let windows = TimeWindow::from_json(json).unwrap();
        assert_eq!(windows[0].days, vec![1, 2, 3, 4, 5]);
    }

    #[test]
    fn test_time_window_no_windows_key() {
        let windows = TimeWindow::from_json(r#"{}"#).unwrap();
        assert!(windows.is_empty());
    }

    #[test]
    fn test_time_window_is_active_day_match() {
        let dt = DateTime::parse_from_rfc3339("2026-07-17T10:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(dt.weekday().num_days_from_sunday(), 5);

        let w = TimeWindow {
            start_hour: 9,
            end_hour: 17,
            days: vec![5],
        };
        assert!(w.is_active(dt));
    }

    #[test]
    fn test_time_window_not_active_wrong_day() {
        let dt = DateTime::parse_from_rfc3339("2026-07-17T10:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        let w = TimeWindow {
            start_hour: 9,
            end_hour: 17,
            days: vec![1, 2, 3, 4],
        };
        assert!(!w.is_active(dt));
    }

    #[test]
    fn test_time_window_all_days_active() {
        let dt = DateTime::parse_from_rfc3339("2026-07-17T10:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        let w = TimeWindow {
            start_hour: 9,
            end_hour: 17,
            days: vec![],
        };
        assert!(w.is_active(dt));
    }

    #[test]
    fn test_time_window_midnight_wrap_active() {
        let dt = DateTime::parse_from_rfc3339("2026-07-17T23:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        let w = TimeWindow {
            start_hour: 22,
            end_hour: 2,
            days: vec![],
        };
        assert!(w.is_active(dt));
    }

    #[test]
    fn test_time_window_midnight_wrap_early() {
        let dt = DateTime::parse_from_rfc3339("2026-07-18T01:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        let w = TimeWindow {
            start_hour: 22,
            end_hour: 2,
            days: vec![],
        };
        assert!(w.is_active(dt));
    }

    #[test]
    fn test_time_window_midnight_wrap_outside() {
        let dt = DateTime::parse_from_rfc3339("2026-07-18T03:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        let w = TimeWindow {
            start_hour: 22,
            end_hour: 2,
            days: vec![],
        };
        assert!(!w.is_active(dt));
    }

    #[test]
    fn test_time_window_not_active_outside_hours() {
        let dt = DateTime::parse_from_rfc3339("2026-07-17T20:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        let w = TimeWindow {
            start_hour: 9,
            end_hour: 17,
            days: vec![],
        };
        assert!(!w.is_active(dt));
    }

    #[test]
    fn test_evaluate_all_pass() {
        let app_id = AppId::new("test.app").unwrap();
        let policies = vec![
            make_policy(1, PolicyKind::TimeLimit, true, Some(3600)),
            make_policy(2, PolicyKind::Notify, true, Some(7200)),
        ];
        let verdict = evaluate(&app_id, &policies, 100, false);
        assert!(matches!(verdict, PolicyVerdict::Ok));
    }

    #[test]
    fn test_evaluate_empty_policies() {
        let app_id = AppId::new("test.app").unwrap();
        let verdict = evaluate(&app_id, &[], 0, false);
        assert!(matches!(verdict, PolicyVerdict::Ok));
    }

    #[test]
    fn test_evaluate_block_unconditional() {
        let app_id = AppId::new("test.app").unwrap();
        let policies = vec![make_policy(1, PolicyKind::Block, true, None)];
        let verdict = evaluate(&app_id, &policies, 0, false);
        assert!(matches!(
            verdict,
            PolicyVerdict::Block {
                reason: BlockReason::AppBlock,
                ..
            }
        ));
    }

    #[test]
    fn test_evaluate_block_category() {
        let app_id = AppId::new("test.app").unwrap();
        let policies = vec![make_policy(1, PolicyKind::Block, false, None)];
        let verdict = evaluate(&app_id, &policies, 0, false);
        assert!(matches!(
            verdict,
            PolicyVerdict::Block {
                reason: BlockReason::CategoryBlock,
                ..
            }
        ));
    }

    #[test]
    fn test_evaluate_time_limit_block() {
        let app_id = AppId::new("test.app").unwrap();
        let policies = vec![make_policy(1, PolicyKind::TimeLimit, true, Some(3600))];
        let verdict = evaluate(&app_id, &policies, 4000, false);
        assert!(matches!(
            verdict,
            PolicyVerdict::Block {
                reason: BlockReason::AppTimeLimit,
                ..
            }
        ));
    }

    #[test]
    fn test_evaluate_time_limit_category_block() {
        let app_id = AppId::new("test.app").unwrap();
        let policies = vec![make_policy(1, PolicyKind::TimeLimit, false, Some(3600))];
        let verdict = evaluate(&app_id, &policies, 4000, false);
        assert!(matches!(
            verdict,
            PolicyVerdict::Block {
                reason: BlockReason::CategoryTimeLimit,
                ..
            }
        ));
    }

    #[test]
    fn test_evaluate_notify_exceeded() {
        let app_id = AppId::new("test.app").unwrap();
        let policies = vec![make_policy(1, PolicyKind::Notify, true, Some(3600))];
        let verdict = evaluate(&app_id, &policies, 4000, false);
        assert!(matches!(verdict, PolicyVerdict::Notify { .. }));
    }

    #[test]
    fn test_evaluate_block_wins_over_notify() {
        let app_id = AppId::new("test.app").unwrap();
        let policies = vec![
            make_policy(1, PolicyKind::Notify, true, Some(3600)),
            make_policy(2, PolicyKind::Block, true, None),
        ];
        let verdict = evaluate(&app_id, &policies, 4000, false);
        assert!(matches!(verdict, PolicyVerdict::Block { .. }));
    }

    #[test]
    fn test_evaluate_block_wins_over_time_limit() {
        let app_id = AppId::new("test.app").unwrap();
        let policies = vec![
            make_policy(1, PolicyKind::Block, true, None),
            make_policy(2, PolicyKind::TimeLimit, true, Some(100)),
        ];
        let verdict = evaluate(&app_id, &policies, 0, false);
        assert!(matches!(verdict, PolicyVerdict::Block { .. }));
    }

    #[test]
    fn test_evaluate_first_block_wins() {
        let app_id = AppId::new("test.app").unwrap();
        let policies = vec![
            make_policy(1, PolicyKind::TimeLimit, true, Some(100)),
            make_policy(2, PolicyKind::Block, false, None),
        ];
        let verdict = evaluate(&app_id, &policies, 200, false);
        assert!(matches!(
            verdict,
            PolicyVerdict::Block { policy_id, .. } if policy_id == PolicyId(1)
        ));
    }

    #[test]
    fn test_evaluate_inactive_policy_skipped() {
        let app_id = AppId::new("test.app").unwrap();
        let p = make_policy(1, PolicyKind::Block, true, None);
        let policies = match p {
            PolicyConfig::Block {
                id,
                app_id,
                category_id,
                ..
            } => {
                vec![PolicyConfig::Block {
                    id,
                    app_id,
                    category_id,
                    active: false,
                }]
            }
            _ => unreachable!(),
        };
        let verdict = evaluate(&app_id, &policies, 0, false);
        assert!(matches!(verdict, PolicyVerdict::Ok));
    }

    #[test]
    fn test_evaluate_notify_with_repeat() {
        let app_id = AppId::new("test.app").unwrap();
        let policies = vec![make_policy_full(
            1,
            PolicyKind::Notify,
            true,
            Some(3600),
            0,
            Some(300),
            true,
        )];
        let verdict = evaluate(&app_id, &policies, 4000, false);
        assert!(matches!(
            verdict,
            PolicyVerdict::Notify {
                repeat_interval: Some(300),
                ..
            }
        ));
    }

    #[test]
    fn test_evaluate_time_limit_at_exact_boundary() {
        let app_id = AppId::new("test.app").unwrap();
        let policies = vec![make_policy(1, PolicyKind::TimeLimit, true, Some(3600))];
        let verdict = evaluate(&app_id, &policies, 3600, false);
        assert!(matches!(verdict, PolicyVerdict::Block { .. }));
    }

    #[test]
    fn test_evaluate_notify_at_exact_boundary() {
        let app_id = AppId::new("test.app").unwrap();
        let policies = vec![make_policy(1, PolicyKind::Notify, true, Some(3600))];
        let verdict = evaluate(&app_id, &policies, 3600, false);
        assert!(matches!(verdict, PolicyVerdict::Notify { .. }));
    }

    #[test]
    #[should_panic(expected = "Block policy has no tracked state")]
    fn test_app_state_block_panics() {
        let policy = make_policy(1, PolicyKind::Block, true, None);
        app_state((0, false), &policy);
    }

    #[test]
    fn test_app_state_time_limit_normal() {
        let policy = make_policy(1, PolicyKind::TimeLimit, true, Some(3600));
        let state = app_state((100, false), &policy);
        match state {
            TrackedApp::TimeLimited(app) => {
                assert_eq!(app.remaining(), 3500);
                assert!(app.can_extend());
                assert_eq!(app.effective_limit(), 3600);
            }
            _ => panic!("expected TimeLimited"),
        }
    }

    #[test]
    fn test_app_state_time_limit_extended() {
        let policy = PolicyConfig::TimeLimit {
            id: PolicyId(1),
            app_id: Some(AppId::new("test.app").unwrap()),
            category_id: None,
            time_limit_seconds: 3600,
            extra_seconds: 600,
            active: true,
        };
        let state = app_state((4000, true), &policy);
        match state {
            TrackedApp::TimeLimited(app) => {
                assert_eq!(app.remaining(), 200);
                assert!(!app.can_extend());
                assert_eq!(app.effective_limit(), 4200);
            }
            _ => panic!("expected TimeLimited"),
        }
    }

    #[test]
    fn test_app_state_notify() {
        let policy = make_policy(1, PolicyKind::Notify, true, Some(3600));
        let state = app_state((100, false), &policy);
        match state {
            TrackedApp::TimeTracked(app) => {
                assert_eq!(app.remaining(), 3500);
                assert!(!app.is_exceeded());
            }
            _ => panic!("expected TimeTracked"),
        }
    }

    #[test]
    fn test_policy_config_from_domain_policy_full() {
        let p = wellbeing_core::Policy {
            id: PolicyId(42),
            name: "Test".into(),
            kind: PolicyKind::TimeLimit,
            app_id: "firefox".into(),
            category_id: 0,
            time_limit_seconds: 3600,
            extra_seconds: 300,
            notification_repeat_interval_seconds: 0,
            schedule_json: String::new(),
            active: true,
            created_by: 1000,
            owner_id: 1000,
            created_at: "2026-01-01".into(),
            updated_at: "2026-01-01".into(),
        };

        let cfg: PolicyConfig = p.into();
        match &cfg {
            PolicyConfig::TimeLimit {
                id,
                app_id,
                category_id,
                time_limit_seconds,
                extra_seconds,
                active,
            } => {
                assert_eq!(*id, PolicyId(42));
                assert_eq!(app_id.as_ref().unwrap().as_str(), "firefox");
                assert!(category_id.is_none());
                assert_eq!(*time_limit_seconds, 3600);
                assert_eq!(*extra_seconds, 300);
                assert!(*active);
            }
            _ => panic!("expected TimeLimit"),
        }
    }

    #[test]
    fn test_policy_config_empty_app_id_and_sentinels() {
        let p = wellbeing_core::Policy {
            id: PolicyId(1),
            name: "CatBlock".into(),
            kind: PolicyKind::Block,
            app_id: String::new(),
            category_id: 5,
            time_limit_seconds: 0,
            extra_seconds: 0,
            notification_repeat_interval_seconds: 0,
            schedule_json: String::new(),
            active: true,
            created_by: 1000,
            owner_id: 1000,
            created_at: "".into(),
            updated_at: "".into(),
        };
        let cfg: PolicyConfig = p.into();
        match &cfg {
            PolicyConfig::Block {
                app_id,
                category_id,
                ..
            } => {
                assert!(app_id.is_none());
                assert_eq!(*category_id, Some(CategoryId(5)));
            }
            _ => panic!("expected Block"),
        }
    }

    #[test]
    fn test_filter_policies_empty_schedule_kept() {
        let p = wellbeing_core::Policy {
            id: PolicyId(1),
            name: "AlwaysActive".into(),
            kind: PolicyKind::TimeLimit,
            app_id: "test".into(),
            category_id: 0,
            time_limit_seconds: 3600,
            extra_seconds: 0,
            notification_repeat_interval_seconds: 0,
            schedule_json: String::new(),
            active: true,
            created_by: 1000,
            owner_id: 1000,
            created_at: "".into(),
            updated_at: "".into(),
        };
        let result = filter_policies_by_schedule(vec![p], Utc::now());
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn test_filter_policies_with_schedule_active() {
        let json = r#"{"time_windows": [{"start_hour": 0, "end_hour": 23}]}"#;
        let p = wellbeing_core::Policy {
            id: PolicyId(1),
            name: "Scheduled".into(),
            kind: PolicyKind::Block,
            app_id: "test".into(),
            category_id: 0,
            time_limit_seconds: 0,
            extra_seconds: 0,
            notification_repeat_interval_seconds: 0,
            schedule_json: json.into(),
            active: true,
            created_by: 1000,
            owner_id: 1000,
            created_at: "".into(),
            updated_at: "".into(),
        };
        let result = filter_policies_by_schedule(vec![p], Utc::now());
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn test_filter_policies_with_schedule_inactive() {
        let json = r#"{"time_windows": [{"start_hour": 0, "end_hour": 1}]}"#;
        let p = wellbeing_core::Policy {
            id: PolicyId(1),
            name: "NightOnly".into(),
            kind: PolicyKind::Block,
            app_id: "test".into(),
            category_id: 0,
            time_limit_seconds: 0,
            extra_seconds: 0,
            notification_repeat_interval_seconds: 0,
            schedule_json: json.into(),
            active: true,
            created_by: 1000,
            owner_id: 1000,
            created_at: "".into(),
            updated_at: "".into(),
        };
        let now = DateTime::parse_from_rfc3339("2026-07-17T10:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let result = filter_policies_by_schedule(vec![p], now);
        assert_eq!(result.len(), 0);
    }
}
