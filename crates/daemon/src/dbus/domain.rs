//! Domain types for the D-Bus interface module.

use wellbeing_core::{AppId, BlockedAppEntry, Uid};

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Blocked apps shared across the daemon.
pub(crate) type BlockedAppsMap = Arc<RwLock<HashMap<Uid, HashMap<AppId, BlockedAppEntry>>>>;
