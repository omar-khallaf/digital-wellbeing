//! Policies management screen — ViewModel, form types, and gpui components.
//!
//! Data flow: Repository → `build_policies_viewmodel()` → `PoliciesViewModel`
//! → `render_policies()` (gpui element tree).

pub mod data;
mod domain;
mod ui;

pub use data::*;
pub use domain::*;
pub use ui::*;
