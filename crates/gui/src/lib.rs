//! wellbeing-gui — Digital Wellbeing desktop UI.
//!
//! Binary structure:
//! - `main.rs` — entry point (gpui::run + background tokio)
//! - `app.rs` — app shell (TitleBar, TabBar, Admin/User mode)
//! - `dbus/` — `DaemonClient` + `SignalCoalescer` + signal subscription
//! - `cache/` — `ClientCache<K,V>` simple in-memory dedup cache
//! - `dashboard/` — Dashboard ViewModel + gpui components
//! - `policies/` — Policies ViewModel + gpui components
//! - `reports/` — Reports ViewModel + gpui components

pub mod app;
pub mod appshell;
pub mod cache;
pub mod chart;
pub mod components;
pub mod dashboard;
pub mod dbus;
pub mod policies;
pub mod reports;
pub mod theme;
