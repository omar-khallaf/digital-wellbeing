//! App shell domain types — pure data structures with no GPUI dependency.
//!
//! These types define the shell-level wiring between dashboard, policies,
//! and reports screens. `RenderMode` and `Tab` are the top-level navigation
//! primitives; `AppState` is a minimal shared state (just connection info
//! and selected range — all data lives in the per-flow ViewModels).

use std::sync::Arc;

use tokio::sync::RwLock;
use wellbeing_core::DateRange;

use crate::dashboard;
use crate::dbus;
use crate::policies;
use crate::reports;

/// Runtime mode determined by getuid().
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderMode {
    Admin,
    User,
}

impl RenderMode {
    pub fn detect() -> Self {
        if nix::unistd::Uid::current().is_root() {
            RenderMode::Admin
        } else {
            RenderMode::User
        }
    }

    pub fn is_admin(&self) -> bool {
        matches!(self, RenderMode::Admin)
    }
}

/// Active tab in the app shell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Dashboard,
    Policies,
    Reports,
}

impl Tab {
    pub fn all() -> &'static [Tab] {
        &[Tab::Dashboard, Tab::Policies, Tab::Reports]
    }

    pub fn label(&self) -> &'static str {
        match self {
            Tab::Dashboard => "Dashboard",
            Tab::Policies => "Policies",
            Tab::Reports => "Reports",
        }
    }

    pub fn icon(&self) -> &'static str {
        match self {
            Tab::Dashboard => "\u{25cf}",
            Tab::Policies => "\u{2699}",
            Tab::Reports => "\u{1f4ca}",
        }
    }
}

/// Bundle of ViewModels sent from the background flows to the GPUI entity
/// on each data change — keeps the foreground render path single-pass.
///
/// Each field is `Option` so a flow can signal "data unavailable" (daemon
/// disconnected) without blocking the other screens.
#[derive(Debug, Clone)]
pub struct AppViewModels {
    pub dashboard: Option<dashboard::DashboardViewModel>,
    pub policies: Option<policies::PoliciesViewModel>,
    pub reports: Option<reports::ReportsViewModel>,
}

/// Minimal shared state between the GPUI entity and background flows.
///
/// No data caches — all screen data lives in per-flow ViewModels emitted
/// through channels.  This struct only carries the date range (which both
/// the dashboard and reports flows need) and connection metadata for the
/// status banner.
pub struct AppState {
    pub mode: RenderMode,
    pub uid: u32,
    pub selected_range: Arc<RwLock<DateRange>>,
    pub daemon_available: Arc<RwLock<bool>>,
    pub connection_status: Arc<RwLock<dbus::ConnectionStatus>>,
}
