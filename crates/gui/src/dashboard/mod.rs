//! Dashboard screen — daily usage overview with charts and app list.
//!
//! Data flow: D-Bus cache → `build_dashboard_viewmodel()` → `DashboardViewModel`
//! → `render_dashboard_view()` (gpui element tree).

mod domain;
mod timeline;
mod ui;
mod viewmodel;

pub use domain::*;
pub use timeline::*;
pub use ui::*;
pub use viewmodel::*;
