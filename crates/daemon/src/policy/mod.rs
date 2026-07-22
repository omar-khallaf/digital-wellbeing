//! Policy evaluation and configuration.
//!
//! Contains [`PolicyConfig`], [`PolicyVerdict`], and [`evaluate`] for
//! gate-first enforcement decisions. Data access is delegated to
//! `data/` via [`PolicyRepo`].

mod core;
pub mod data;
mod domain;

pub use core::*;
pub(crate) use data::*;
pub use domain::*;
