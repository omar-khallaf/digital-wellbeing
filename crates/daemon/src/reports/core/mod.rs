//! Report generation and data aggregation functions — core module.
//!
//! Sub-modules:
//! - `queries` — individual date queries (daily usage, event range, etc.)
//! - `aggregation` — hourly aggregation and startup recovery
//! - `pruning` — periodic old-data deletion loop

mod aggregation;
mod pruning;
mod queries;

pub use aggregation::*;
pub use pruning::*;
pub use queries::*;
