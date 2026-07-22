//! Blocking enforcement module.
//!
//! [`EnforcerActor`] is the core enforcement engine. It receives
//! [`PlatformEvent`]s, evaluates policies **gate-first** (before any event
//! is persisted), and manages overlay state declaratively.
//!
//! ## Gate-First Discipline
//!
//! On `WindowFocused`, the actor resolves categories & policies, calls
//! [`evaluate`], and acts on the verdict **before** writing any event:
//!
//! - **Block** → record in active_blocks, DON'T write `WindowFocused` for the app.
//!   The previous app's interval IS closed (`Unfocused` written).
//! - **Notify** → write events normally, send notification, start repeat timer.
//! - **Ok** → write events normally, start limit timer.
//!
//! ## Declarative Block State
//!
//! Block state is maintained in `active_blocks` and exposed to the compositor
//! plugin via the `BlockStateChanged` D-Bus signal. The daemon never commands
//! the plugin directly — the plugin reads daemon state and manages its own
//! overlays.

mod core;
mod data;
mod domain;

pub use core::*;
pub use domain::*;
