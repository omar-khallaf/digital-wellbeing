//! Policy evaluation and configuration.
//!
//! Contains [`evaluate`] for priority-ordered first-match-wins policy
//! evaluation. Data access is delegated to `store/repos/` via
//! [`crate::store::daos::policies::PolicyRepo`].

mod core;
pub mod data;
mod domain;

pub use core::*;
pub use domain::*;
