// rust-lang/rust#159228: solver recursion guard trips on the zbus
// #[interface] dispatch future through turso transactions.
// Compiler-implementation quirk, not a code defect.
#![allow(recursion_depth_exceeding_limit)]

//! wellbeing-daemon library crate root.
//! Re-exports all internal modules for integration testing.

pub mod blocking;
pub mod bus_resolution;
pub mod categorization;
pub mod dbus;
pub mod logind;
pub mod platform;
pub mod policy;
pub mod reports;
pub mod signal;
pub mod store;
