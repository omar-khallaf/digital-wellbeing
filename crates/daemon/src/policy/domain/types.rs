//! Domain types for the policy evaluation engine.
//!
//! These types represent the redesigned policy model: priority-ordered,
//! first-match-wins evaluation with `Notify` as non-terminating, `Allow`
//! explicit effect, and `Target::Any` wildcard.

use wellbeing_core::{AppClass, Category, DomainPattern, PolicyId, TimeWindow};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    Allow,
    Block,
    TimeLimit { limit_minutes: u64 },
    Notify { limit_minutes: u64 },
}

impl Effect {
    pub fn is_terminating(self) -> bool {
        !matches!(self, Effect::Notify { .. })
    }

    /// Map to the D-Bus wire discriminant.
    pub fn kind_discriminant(&self) -> wellbeing_core::Effect {
        match self {
            Effect::Allow => wellbeing_core::Effect::Allow,
            Effect::Block => wellbeing_core::Effect::Block,
            Effect::TimeLimit { .. } => wellbeing_core::Effect::TimeLimit,
            Effect::Notify { .. } => wellbeing_core::Effect::Notify,
        }
    }
}

/// `Category` stores the [`Category`] enum variant directly — the old
/// `categories` table has been removed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyTarget {
    App(AppClass),
    Category(Category),
    Domain(DomainPattern),
    Any,
}

/// Policies are sorted by priority before evaluation.
/// An empty `schedule` means always active.
#[derive(Debug, Clone)]
pub struct Policy {
    pub id: PolicyId,
    pub name: String,
    pub effect: Effect,
    pub target: PolicyTarget,
    pub priority: u64,
    /// Empty vec = always active. Each window is checked with OR logic.
    pub schedule: Vec<TimeWindow>,
    pub time_limit_minutes: u64,
    pub user_id: u32,
    pub created_by: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvalResult {
    /// The first terminating policy (Allow/Block/TimeLimit) that matched.
    /// `None` means no policy matched (unrestricted).
    pub terminating: Option<(PolicyId, Effect)>,
    /// All matching Notify effects, in priority order.
    /// These are non-terminating — the interval continues.
    pub notifies: Vec<(PolicyId, Effect)>,
}

impl EvalResult {
    pub fn is_unrestricted(&self) -> bool {
        self.terminating.is_none() && self.notifies.is_empty()
    }
}
