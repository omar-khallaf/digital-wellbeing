//! App shell domain types — pure data structures with no GPUI dependency.
//!
//! These types define the shell-level wiring between dashboard, policies,
//! and reports screens. `RenderMode` and `Tab` are the top-level navigation
//! primitives; `AppState` is the shared mutable cache behind the GPUI entity.

use wellbeing_core::*;

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
    /// All available tabs in display order.
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

/// Bundle of ViewModels sent from the background refresh loop to the GPUI
/// entity on each data change — keeps the foreground render path single-pass.
#[derive(Debug, Clone)]
pub struct AppViewModels {
    pub dashboard: Option<dashboard::DashboardViewModel>,
    pub policies: Option<policies::PoliciesViewModel>,
    pub reports: Option<reports::ReportsViewModel>,
}

/// Shared state accessible by all screen views.
pub struct AppState {
    pub mode: RenderMode,
    pub uid: u32,
    pub client: crate::dbus::DaemonClient,
    pub selected_range: DateRange,
    pub range_cache: Vec<DailySummary>,
    pub policy_cache: Vec<PolicyData>,
    pub category_cache: Vec<Category>,
    pub app_category_cache: Vec<AppCategoryRow>,
    pub block_cards: Vec<dashboard::BlockCardInfo>,
    pub day_events_cache: Vec<DayEventRow>,
    pub daemon_available: bool,
    pub connection_status: dbus::ConnectionStatus,
}
