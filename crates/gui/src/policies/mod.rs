//! Policies management screen — ViewModel, form types, and gpui components.
//!
//! Data flow: D-Bus cache → `build_policies_viewmodel()` → `PoliciesViewModel`
//! → `render_policies()` (gpui element tree).

mod data;
mod domain;
mod ui;

pub use data::*;
pub use domain::*;
