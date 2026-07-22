//! D-Bus interface controller — holds shared state and dependencies.

mod controller;
mod core;
mod data;
mod domain;
mod methods;
mod signals;

pub use controller::DaemonInterface;
