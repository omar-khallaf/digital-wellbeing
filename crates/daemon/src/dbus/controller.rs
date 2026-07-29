//! D-Bus interface controller — holds shared state and dependencies.

use std::sync::Arc;

use tokio::sync::{RwLock, mpsc, mpsc::UnboundedSender};
use wellbeing_core::Clock;

use crate::blocking::InternalEvent;
use crate::categorization::data::CategorizationRepo;
use crate::platform::PlatformEvent;
use crate::platform::linux::PluginRegistry;
use crate::policy::data::PolicyRepo;
use crate::reports::data::ReportsRepo;

use super::domain::BlockedAppsMap;

/// Configuration bundle for constructing a [`DaemonInterface`].
///
/// All fields are required — use the struct literal directly:
/// ```ignore
/// DaemonInterface::new(DaemonInterfaceConfig {
///     policy_repo,
///     categorization_repo,
///     reports_repo,
///     registry,
///     event_tx: platform.event_tx(),
///     clock: Box::new(SystemClock),
///     blocked_apps,
///     tokio_handle: tokio::runtime::Handle::current(),
///     policy_tx,
/// })
/// ```
pub struct DaemonInterfaceConfig {
    pub policy_repo: PolicyRepo,
    pub categorization_repo: CategorizationRepo,
    pub reports_repo: ReportsRepo,
    pub registry: Arc<RwLock<PluginRegistry>>,
    pub event_tx: UnboundedSender<PlatformEvent>,
    pub clock: Box<dyn Clock>,
    pub blocked_apps: BlockedAppsMap,
    pub tokio_handle: tokio::runtime::Handle,
    pub policy_tx: mpsc::Sender<InternalEvent>,
}

/// The main D-Bus interface object, registered on the bus as
/// `org.wellbeing.v1.Controller`.
pub struct DaemonInterface {
    pub(crate) policy_repo: PolicyRepo,
    pub(crate) categorization_repo: CategorizationRepo,
    pub(crate) reports_repo: ReportsRepo,
    pub(crate) registry: Arc<RwLock<PluginRegistry>>,
    pub(crate) event_tx: UnboundedSender<PlatformEvent>,
    pub(crate) clock: Box<dyn Clock>,
    pub(crate) blocked_apps: BlockedAppsMap,
    pub(crate) policy_tx: mpsc::Sender<InternalEvent>,
    pub(crate) tokio_handle: tokio::runtime::Handle,
}

impl DaemonInterface {
    pub fn new(config: DaemonInterfaceConfig) -> Self {
        let DaemonInterfaceConfig {
            policy_repo,
            categorization_repo,
            reports_repo,
            registry,
            event_tx,
            clock,
            blocked_apps,
            tokio_handle,
            policy_tx,
        } = config;
        Self {
            policy_repo,
            categorization_repo,
            reports_repo,
            registry,
            event_tx,
            clock,
            blocked_apps,
            policy_tx,
            tokio_handle,
        }
    }
}
