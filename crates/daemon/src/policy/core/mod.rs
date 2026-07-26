//! Business logic for policy evaluation.
//!
//! `evaluate()` is a pure domain function that accepts pre-filtered,
//! pre-sorted policies and returns an `EvalResult`.
//!
//! Schedule filtering is inlined in `evaluate()` — no separate scheduler module.
//! Tracked state is removed — evaluation is stateless (usage passed as arg).

mod evaluator;

pub use evaluator::*;
