//! wellbeing-gui — Digital Wellbeing desktop UI.
//!
//! Binary structure:
//! - `main.rs` — entry point (gpui::run + background tokio)
//! - `app.rs` — app shell (TitleBar, TabBar, Admin/User mode)
//! - `dbus/` — `DaemonClient` + `SignalCoalescer` + signal subscription
//! - `cache/` — `ClientCache<K,V>` stale-while-revalidate
//! - `screens/` — ViewModels + gpui components per screen

pub mod app;
pub mod cache;
pub mod components;
pub mod dbus;
pub mod screens;
pub mod theme;
