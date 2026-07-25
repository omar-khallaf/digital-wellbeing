//! Persistence sub-modules for the blocking/enforcement feature.
//! Split into events, deltas, and queries for modularity.

mod deltas;
mod events;
mod queries;

pub(crate) use events::BlockingRepo;
pub use events::EventRow;
