//! Policy repository — CRUD for focus/restriction policies plus the
//! policy-resolution query used by the blocker.

pub(crate) mod queries;

// All policies access goes through `queries::*` functions.
