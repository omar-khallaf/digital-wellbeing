//! D-Bus proxy — generated from `Daemon` trait via `zbus::proxy`.
//!
//! The `DaemonProxy<'static>` type is the primary way repos connect to the
//! daemon's `org.wellbeing.v1.Controller` interface.

mod proxy;

pub use proxy::DaemonProxy;
