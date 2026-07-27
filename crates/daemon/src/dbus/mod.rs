//! D-Bus interface controller — holds shared state and dependencies.

mod controller;
mod core;
pub mod domain;
mod methods;
mod signals;

pub use controller::{DaemonInterface, DaemonInterfaceConfig};
