//! Shared test helpers for the blocking/enforcement feature.
//!
//! Provides [`MockPlatform`], [`setup`], and [`dt`] used by
//! persistence and core test modules. Only compiled in `#[cfg(test)]`.

#![cfg(test)]

use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, TimeZone, Utc};
use tempfile::TempDir;
use tokio::sync::RwLock;
use tokio::sync::mpsc;
use wellbeing_core::{AppClass, BlockedAppEntry, Uid, VirtualClock};

use crate::blocking::EnforcerActor;
use crate::platform::{Platform, PlatformEvent, PluginRegistry};
use crate::signal::DaemonSignal;
use crate::store::{DbPool, StoreBuilder};

pub struct MockPlatform {
    registry: Arc<RwLock<PluginRegistry>>,
}

impl MockPlatform {
    pub fn new() -> Self {
        Self {
            registry: Arc::new(RwLock::new(PluginRegistry::new())),
        }
    }
}

impl Default for MockPlatform {
    fn default() -> Self {
        Self::new()
    }
}

impl Platform for MockPlatform {
    type EventStream = tokio_stream::wrappers::UnboundedReceiverStream<PlatformEvent>;

    async fn notify(&self, _title: &str, _body: &str) -> anyhow::Result<()> {
        Ok(())
    }

    fn registry(&self) -> Arc<RwLock<PluginRegistry>> {
        self.registry.clone()
    }
}

pub fn dt(secs: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(secs, 0).unwrap()
}

/// Create an EnforcerActor with a real SQLite database and mocks.
pub async fn setup() -> (
    TempDir,
    DbPool,
    EnforcerActor<MockPlatform, VirtualClock>,
    mpsc::UnboundedReceiver<DaemonSignal>,
) {
    let tmp = TempDir::new().expect("temp dir");
    let db_path = tmp.path().join("test.db");
    let pool = StoreBuilder::new(db_path)
        .build()
        .await
        .expect("build store");
    let (signal_tx, signal_rx) = mpsc::unbounded_channel();
    let platform = Arc::new(MockPlatform::new());
    let clock = VirtualClock::new(dt(1_000_000));
    let blocked_apps = Arc::new(RwLock::new(HashMap::<
        Uid,
        HashMap<AppClass, BlockedAppEntry>,
    >::new()));

    let (actor, _) = EnforcerActor::new(pool.clone(), platform, clock, signal_tx, blocked_apps);

    (tmp, pool, actor, signal_rx)
}
