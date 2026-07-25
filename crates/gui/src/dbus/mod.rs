//! GUI D-Bus client: connection management, signal handling, and daemon proxy.
//!
//! Re-exports from three sub-modules:
//! - `bus` — bus selection, daemon presence watching, connection status types
//! - `client` — `DaemonClient` proxy wrapper with response caching
//! - `signals` — `SignalCoalescer` + signal subscription

mod bus;
mod client;
mod signals;

pub use bus::{BusType, ConnectionStatus, DaemonPresenceEvent, spawn_daemon_name_watch};
pub use client::DaemonClient;
pub use signals::{CoalescedNotifications, SignalCoalescer, spawn_signal_listener};
