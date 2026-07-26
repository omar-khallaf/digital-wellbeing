//! Daily usage repository — materialized per-app, per-title, and per-category
//! usage tables plus the full delta/interval computation pipeline.

pub(crate) mod accumulate;
pub(crate) mod deltas;
pub(crate) mod reads;
pub(crate) mod upserts;
