//! D-Bus interface controller — holds shared state and dependencies.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use tokio::sync::{RwLock, mpsc::UnboundedSender};
use wellbeing_core::Clock;

use crate::platform::PlatformEvent;
use crate::platform::linux::PluginRegistry;
use crate::store::DbPool;

use super::domain::ActiveBlocksMap;

/// The main D-Bus interface object, registered on the bus as
/// `org.wellbeing.v1.Controller`.
pub struct DaemonInterface {
    pub(crate) pool: DbPool,
    pub(crate) registry: Arc<RwLock<PluginRegistry>>,
    pub(crate) event_tx: UnboundedSender<PlatformEvent>,
    pub(crate) plugin_reg_cooldown: RwLock<HashMap<u32, Instant>>,
    pub(crate) clock: Box<dyn Clock>,
    pub(crate) active_blocks: ActiveBlocksMap,
    pub(crate) tokio_handle: tokio::runtime::Handle,
}

impl DaemonInterface {
    pub fn new(
        pool: DbPool,
        registry: Arc<RwLock<PluginRegistry>>,
        event_tx: UnboundedSender<PlatformEvent>,
        clock: Box<dyn Clock>,
        active_blocks: ActiveBlocksMap,
        tokio_handle: tokio::runtime::Handle,
    ) -> Self {
        Self {
            pool,
            registry,
            event_tx,
            plugin_reg_cooldown: RwLock::new(HashMap::new()),
            clock,
            active_blocks,
            tokio_handle,
        }
    }
}
