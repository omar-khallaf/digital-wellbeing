//! Domain types for the D-Bus interface module.

use wellbeing_core::{AppClass, BlockedAppEntry, Uid};

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Blocked apps shared across the daemon.
pub type BlockedAppsMap = Arc<RwLock<HashMap<Uid, HashMap<AppClass, BlockedAppEntry>>>>;
