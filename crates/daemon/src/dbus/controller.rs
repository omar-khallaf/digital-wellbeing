//! D-Bus interface controller — holds shared state and dependencies.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use tokio::sync::{RwLock, mpsc, mpsc::UnboundedSender};
use wellbeing_core::Clock;

use crate::blocking::InternalEvent;
use crate::categorization::data::CategorizationRepo;
use crate::platform::PlatformEvent;
use crate::platform::linux::PluginRegistry;
use crate::policy::data::PolicyRepo;
use crate::reports::data::ReportsRepo;

use super::domain::BlockedAppsMap;

/// The main D-Bus interface object, registered on the bus as
/// `org.wellbeing.v1.Controller`.
///
/// Holds shared state and feature repositories — all created once at
/// startup and reused for the lifetime of the process.
pub struct DaemonInterface {
    pub(crate) policy_repo: PolicyRepo,
    pub(crate) categorization_repo: CategorizationRepo,
    pub(crate) reports_repo: ReportsRepo,
    pub(crate) registry: Arc<RwLock<PluginRegistry>>,
    pub(crate) event_tx: UnboundedSender<PlatformEvent>,
    pub(crate) plugin_reg_cooldown: RwLock<HashMap<u32, Instant>>,
    pub(crate) clock: Box<dyn Clock>,
    pub(crate) blocked_apps: BlockedAppsMap,
    pub(crate) policy_tx: mpsc::Sender<InternalEvent>,
    pub(crate) tokio_handle: tokio::runtime::Handle,
}

impl DaemonInterface {
    pub fn new(
        policy_repo: PolicyRepo,
        categorization_repo: CategorizationRepo,
        reports_repo: ReportsRepo,
        registry: Arc<RwLock<PluginRegistry>>,
        event_tx: mpsc::UnboundedSender<PlatformEvent>,
        clock: Box<dyn Clock>,
        blocked_apps: BlockedAppsMap,
        tokio_handle: tokio::runtime::Handle,
        policy_tx: mpsc::Sender<InternalEvent>,
    ) -> Self {
        Self {
            policy_repo,
            categorization_repo,
            reports_repo,
            registry,
            event_tx,
            plugin_reg_cooldown: RwLock::new(HashMap::new()),
            clock,
            blocked_apps,
            policy_tx,
            tokio_handle,
        }
    }
}
