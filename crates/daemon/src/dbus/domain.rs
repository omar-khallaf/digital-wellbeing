//! Domain types for the D-Bus interface module.

use wellbeing_core::{ActiveBlockEntry, AppId, Uid};

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Active blocks shared across the daemon.
pub(crate) type ActiveBlocksMap = Arc<RwLock<HashMap<Uid, HashMap<AppId, ActiveBlockEntry>>>>;
