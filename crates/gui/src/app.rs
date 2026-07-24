//! App shell — sidebar navigation, header, and content routing by tab.
//!
//! This module is now a thin re-export of `appshell/` which follows the
//! feature-per-directory / DDD pattern matching other screens:
//!
//! - `appshell/domain.rs` — `RenderMode`, `Tab`, `AppViewModels`, `AppState`
//! - `appshell/data.rs`   — `App` entity struct + data methods
//! - `appshell/ui.rs`     — `Render` impl + sidebar/header/content helpers
//!
//! All imports from other parts of the crate (`crate::app::*`) continue
//! to work unchanged.

pub use crate::appshell::*;
