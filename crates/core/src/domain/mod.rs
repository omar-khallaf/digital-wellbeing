//! Shared D-Bus wire types used across daemon ↔ GUI interface.
//!
//! These are flat, zvariant-compatible types with sentinel values for
//! optional fields (no `Option<T>`). Daemon-internal domain enums live in
//! their owning feature crates (e.g. `policy/domain.rs`).

mod category_types;
mod day_event;
mod policy_types;
mod usage_types;
mod window_types;

pub use category_types::*;
pub use day_event::*;
pub use policy_types::*;
pub use usage_types::*;
pub use window_types::*;

#[cfg(test)]
mod tests;
