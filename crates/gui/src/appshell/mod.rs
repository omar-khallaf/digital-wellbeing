//! App shell — sidebar navigation, header, and content routing by tab.
//!
//! Organized following the feature-per-directory / DDD pattern established in
//! `dashboard/`, `policies/`, and `reports/`:
//!
//! - `domain.rs` — pure data types (`RenderMode`, `Tab`, `AppViewModels`, `AppState`)
//! - `data.rs`   — `App` entity struct + all data methods (construction, refresh, inputs)
//! - `ui.rs`     — rendering helpers (`sidebar`, `header`, `content_area`, `loading_state`)
//!   plus `impl Render for App`
//!
//! The module is re-exported through `app.rs` so existing imports in `main.rs` and
//! across the crate continue to work unchanged.

mod data;
mod domain;
mod ui;

pub use data::App;
pub use domain::{AppState, AppViewModels, RenderMode, Tab};
