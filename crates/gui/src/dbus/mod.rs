//! GUI D-Bus client: connection management, signal handling, and daemon proxy.
//!
//! Re-exports from two sub-modules:
//! - `bus` — bus selection, daemon presence watching, connection status types
//! - `client` — generated `DaemonProxy` trait

pub mod bus;
pub mod client;

pub use bus::{
    BusManager, BusType, ConnectionStatus, DaemonPresenceEvent, spawn_daemon_presence_broadcast,
};
