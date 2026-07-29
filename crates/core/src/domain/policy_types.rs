use crate::valuetypes::*;
use chrono::{DateTime, Datelike, Timelike, Utc};
use serde::{Deserialize, Serialize};
use serde_repr::{Deserialize_repr, Serialize_repr};
use zvariant::Type;

// ═══════════════════════════════════════════════════════════════════════════
// Effect — policy action discriminant
// ═══════════════════════════════════════════════════════════════════════════

/// Policy action discriminant — maps to DB integer and D-Bus wire.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize_repr, Deserialize_repr, Type)]
pub enum Effect {
    Allow = 0,
    Block = 1,
    TimeLimit = 2,
    Notify = 3,
}

impl From<Effect> for i32 {
    fn from(e: Effect) -> Self {
        e as i32
    }
}

impl TryFrom<i32> for Effect {
    type Error = &'static str;
    fn try_from(v: i32) -> Result<Self, Self::Error> {
        match v {
            0 => Ok(Effect::Allow),
            1 => Ok(Effect::Block),
            2 => Ok(Effect::TimeLimit),
            3 => Ok(Effect::Notify),
            _ => Err("unknown Effect discriminant"),
        }
    }
}

impl Effect {
    /// Whether this effect terminates evaluation (Allow/Block/TimeLimit)
    /// or is non-terminating (Notify).
    pub fn is_terminating(self) -> bool {
        !matches!(self, Effect::Notify)
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// PolicyTarget — what a policy applies to
// ═══════════════════════════════════════════════════════════════════════════

/// Target type discriminant for D-Bus wire (mirrors `PolicyTarget` variants).
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize_repr, Deserialize_repr, Type)]
pub enum TargetType {
    App = 0,
    Category = 1,
    Domain = 2,
    Any = 3,
}

impl From<TargetType> for i32 {
    fn from(t: TargetType) -> Self {
        t as i32
    }
}

impl TryFrom<i32> for TargetType {
    type Error = &'static str;
    fn try_from(v: i32) -> Result<Self, Self::Error> {
        match v {
            0 => Ok(TargetType::App),
            1 => Ok(TargetType::Category),
            2 => Ok(TargetType::Domain),
            3 => Ok(TargetType::Any),
            _ => Err("unknown TargetType discriminant"),
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// TimeWindow — schedule rule
// ═══════════════════════════════════════════════════════════════════════════

/// Single time-window rule for schedule-based policy enforcement.
///
/// `start_minute` and `end_minute` are minutes since midnight (0–1439).
/// `day_mask` is a 7-bit bitmask (bit 0=Sunday … bit 6=Saturday).
/// `0x7F` = all days, `0` = none. Empty schedule (no windows) = always active.
///
/// Cross-midnight: `start_minute > end_minute` means the window spans midnight.
/// `start_minute == end_minute` is invalid (enforced by DB CHECK).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeWindow {
    pub start_minute: u16,
    pub end_minute: u16,
    /// 7-bit day mask: bit 0=Sunday … bit 6=Saturday. 0x7F = all days.
    pub day_mask: u8,
}

impl TimeWindow {
    /// Check if this window is active at `now`.
    pub fn is_active(&self, now: DateTime<Utc>) -> bool {
        // Day check: if day_mask is non-zero, current day must be set.
        if self.day_mask != 0 {
            let day_bit = 1u8 << now.weekday().num_days_from_sunday();
            if self.day_mask & day_bit == 0 {
                return false;
            }
        }
        // Minute-of-day check: supports cross-midnight.
        let minute = (now.hour() * 60 + now.minute()) as u16;
        if self.start_minute < self.end_minute {
            minute >= self.start_minute && minute < self.end_minute
        } else {
            // Cross-midnight: e.g. start=1320(22:00), end=360(06:00)
            minute >= self.start_minute || minute < self.end_minute
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// PolicyData — full policy as exposed over D-Bus
// ═══════════════════════════════════════════════════════════════════════════

/// Full policy as exposed over D-Bus.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct PolicyData {
    pub id: PolicyId,
    pub name: String,
    /// Effect discriminant (0=Allow, 1=Block, 2=TimeLimit, 3=Notify).
    pub effect: Effect,
    /// Target type discriminant (0=App, 1=Category, 2=Domain, 3=Any).
    pub target_type: TargetType,
    /// App identifier — valid when target_type == App.
    pub app_class: AppClass,
    /// Category name — valid when target_type == Category.
    /// The database resolves the name to a row id.
    pub category_name: String,
    /// Domain pattern — valid when target_type == Domain.
    pub domain_pattern: DomainPattern,
    /// Lower priority = evaluated first. Default 100.
    pub priority: i64,
    /// Time limit in minutes — valid when effect is TimeLimit or Notify.
    pub time_limit_minutes: i64,
    /// JSON-serialized `Vec<TimeWindow>`. Empty = always active.
    pub schedule_json: String,
    pub user_id: Uid,
    pub created_by: Uid,
}

/// Input for creating/updating a policy.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct PolicyInput {
    pub name: String,
    /// Effect discriminant (0=Allow, 1=Block, 2=TimeLimit, 3=Notify).
    pub effect: Effect,
    /// Target type discriminant (0=App, 1=Category, 2=Domain, 3=Any).
    pub target_type: TargetType,
    pub app_class: AppClass,
    /// Category name — valid when target_type == Category.
    pub category_name: String,
    pub domain_pattern: DomainPattern,
    pub priority: i64,
    pub time_limit_minutes: i64,
    /// JSON-serialized `Vec<TimeWindow>`. Empty = always active.
    pub schedule_json: String,
    pub user_id: Uid,
}
