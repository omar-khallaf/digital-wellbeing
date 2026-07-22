//! Dashboard screen — daily usage overview with charts and app list.
//!
//! Data flow: D-Bus cache → `build_dashboard_viewmodel()` → `DashboardViewModel`
//! → `render_dashboard_view()` (gpui element tree).

mod data;
mod domain;
mod ui;

pub use data::*;
pub use domain::*;
pub use ui::*;
