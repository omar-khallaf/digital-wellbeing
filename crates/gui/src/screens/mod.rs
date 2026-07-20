//! Screen modules — each screen defines its own ViewModel + gpui component tree.
//!
//! ViewModels are `Send + 'static` structs with zero gpui types. The render
//! loop follows: **Collect** (cache/D-Bus → raw data) → **Transform** (→ ViewModel)
//! → **Render** (→ gpui).

pub mod dashboard;
pub mod policies;
pub mod reports;

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
            Tab::Dashboard => "\u{25cf}", // filled circle
            Tab::Policies => "\u{2699}",  // gear
            Tab::Reports => "\u{1f4ca}",  // bar chart
        }
    }
}
