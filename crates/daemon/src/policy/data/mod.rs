//! Data access layer for policy module.

pub(crate) mod insert;
pub(crate) mod models;
pub(crate) mod repo;

pub use repo::PolicyRepo;
