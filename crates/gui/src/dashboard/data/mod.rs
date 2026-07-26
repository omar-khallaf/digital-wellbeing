//! Dashboard data layer — repository and background flow.

mod flow;
mod repo;

pub use flow::{FlowState, spawn_dashboard_flow};
pub use repo::DashboardRepo;
