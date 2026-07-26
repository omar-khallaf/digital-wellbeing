//! Blocking enforcement engine — orchestration layer.
//!
//! [`EnforcerActor`] receives [`PlatformEvent`]s from the plugin, buffers
//! them for batch persistence, and evaluates policies from the database at
//! minute-tick boundaries.  The plugin is the sole source of truth for
//! current window focus state; the daemon does NOT maintain a
//! `current_focus` map.

mod buffer;
mod handlers;

pub(crate) use buffer::EventBuffer;

use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use diesel_async::AsyncConnection;
use tokio::sync::mpsc;
use tokio::sync::oneshot;
use tracing::{error, info};

use wellbeing_core::*;

use crate::platform::linux::PluginRegistry;
use crate::platform::{Platform, PlatformEvent};
use crate::policy::{PolicyConfig, PolicyVerdict, evaluate, filter_policies_by_schedule};
use crate::signal::DaemonSignal;
use crate::store::DbPool;

use super::data::BlockingRepo;

/// Internal events for the blocking actor.
pub enum InternalEvent {
    /// Flush the event buffer. The optional oneshot sender is signaled
    /// after the flush completes, allowing callers (e.g. shutdown) to
    /// await completion.
    Flush(Option<oneshot::Sender<()>>),
}

/// Core enforcement actor, generic over [`Platform`] and [`Clock`].
pub struct EnforcerActor<P: Platform, C: Clock> {
    pub(crate) repo: BlockingRepo,
    platform: Arc<P>,
    pub(crate) registry: Arc<tokio::sync::RwLock<PluginRegistry>>,
    pub(crate) blocked_apps:
        Arc<tokio::sync::RwLock<HashMap<Uid, HashMap<AppId, BlockedAppEntry>>>>,
    pub(crate) clock: C,
    signal_tx: mpsc::UnboundedSender<DaemonSignal>,
    pub(crate) event_buffer: EventBuffer,
    internal_tx: mpsc::Sender<InternalEvent>,
}

impl<P: Platform, C: Clock> EnforcerActor<P, C> {
    pub fn new(
        pool: DbPool,
        platform: Arc<P>,
        registry: Arc<tokio::sync::RwLock<PluginRegistry>>,
        clock: C,
        signal_tx: mpsc::UnboundedSender<DaemonSignal>,
        blocked_apps: Arc<tokio::sync::RwLock<HashMap<Uid, HashMap<AppId, BlockedAppEntry>>>>,
    ) -> (Self, mpsc::Receiver<InternalEvent>) {
        let (internal_tx, internal_rx) = mpsc::channel::<InternalEvent>(32);

        (
            Self {
                repo: BlockingRepo::new(pool),
                platform,
                registry,
                blocked_apps,
                clock,
                signal_tx,
                event_buffer: EventBuffer::default(),
                internal_tx,
            },
            internal_rx,
        )
    }

    /// Returns a cloneable sender for sending [`InternalEvent`] signals
    /// (used by main.rs minute-ticker to dispatch `InternalEvent::Flush`).
    pub fn flush_handle(&self) -> mpsc::Sender<InternalEvent> {
        self.internal_tx.clone()
    }

    /// Main actor loop. Listens for both platform events and internal
    /// events (flush requests) so that shutdown flushes are processed
    /// even when no platform events are arriving.
    pub async fn run(
        &mut self,
        mut enforcer_rx: mpsc::Receiver<PlatformEvent>,
        mut internal_rx: mpsc::Receiver<InternalEvent>,
    ) {
        loop {
            tokio::select! {
                event = enforcer_rx.recv() => {
                    match event {
                        Some(event) => self.handle_event(event).await,
                        None => {
                            self.drain_remaining(&mut internal_rx).await;
                            break;
                        }
                    }
                }
                internal = internal_rx.recv() => {
                    match internal {
                        Some(event) => self.handle_internal_event(event, "Timer-triggered flush failed").await,
                        None => {
                            info!("enforcer actor: internal event channel closed");
                            break;
                        }
                    }
                }
            }
        }
    }

    /// Drain remaining internal events after the platform channel closes.
    async fn drain_remaining(&mut self, internal_rx: &mut mpsc::Receiver<InternalEvent>) {
        while let Ok(event) = internal_rx.try_recv() {
            self.handle_internal_event(event, "Final flush failed during shutdown")
                .await;
        }
    }

    async fn handle_internal_event(&mut self, event: InternalEvent, error_msg: &str) {
        match event {
            InternalEvent::Flush(ack) => {
                if let Err(e) = self.flush_buffer().await {
                    error!(error = %e, "{}", error_msg);
                } else if let Err(e) = self.evaluate_and_enforce(self.clock.now()).await {
                    error!(error = %e, "Policy evaluation failed on minute-tick");
                }
                if let Some(tx) = ack {
                    let _ = tx.send(());
                }
            }
        }
    }

    /// Flush buffered events to the database, apply closed-interval deltas
    /// from the buffer, and refresh open intervals — in a single transaction.
    ///
    /// All affected UIDs (from events and registered plugins) receive a
    /// [`DaemonSignal::DailyUsageChanged`] so the daily-usage counter advances
    /// even when no focus-switch events arrive.
    pub async fn flush_buffer(&mut self) -> anyhow::Result<()> {
        let now = self.clock.now();

        // Collect affected uids once — before draining the buffer.
        let event_uids = self.event_buffer.uids();
        let all_uids = {
            let mut set: std::collections::HashSet<Uid> = event_uids.iter().copied().collect();
            set.extend(self.registry.read().await.registered_uids());
            set.into_iter().collect::<Vec<_>>()
        };

        let events = self.event_buffer.drain();
        let mut conn = self.repo.pool.get().await?;

        conn.transaction(async |conn| {
            if !events.is_empty() {
                self.repo
                    .apply_closed_deltas_from_buffer(conn, &events, &event_uids, now)
                    .await?;
                self.repo.flush_events(conn, &events).await?;
            }
            BlockingRepo::refresh_open_intervals(conn, &all_uids, now).await?;
            Ok::<_, anyhow::Error>(())
        })
        .await?;

        self.emit_daily_usage_changed(&all_uids);

        Ok(())
    }

    /// Emit [`DaemonSignal::DailyUsageChanged`] for every UID whose usage
    /// may have changed during this flush cycle.
    fn emit_daily_usage_changed(&self, uids: &[Uid]) {
        for uid in uids {
            let _ = self
                .signal_tx
                .send(DaemonSignal::DailyUsageChanged { uid: uid.0 });
        }
    }

    /// Evaluate policies for all currently focused apps and enforce blocks.
    ///
    /// Queries the plugin registry for each registered uid's current focus
    /// since the plugin is the sole source of truth for window state.
    async fn evaluate_and_enforce(&mut self, now: DateTime<Utc>) -> anyhow::Result<()> {
        let uids = self.registry.read().await.registered_uids();

        for uid in uids {
            let focused = {
                let reg = self.registry.read().await;
                reg.current_focus_for_uid(uid).await
            };
            let Some(event) = focused else {
                continue;
            };
            let Some(app_id) = event.app_id() else {
                continue;
            };

            if let Ok(usage_ms) = self.repo.fetch_usage(app_id, uid, &self.clock).await {
                let categories = match self.repo.fetch_categories(app_id, uid).await {
                    Ok(c) => c,
                    Err(e) => {
                        tracing::warn!(%app_id, error = %e, "Failed to fetch categories");
                        Vec::new()
                    }
                };
                let policies = match self
                    .resolve_filtered_policies(app_id, &categories, uid)
                    .await
                {
                    Ok(p) => p,
                    Err(e) => {
                        tracing::warn!(%app_id, error = %e, "Failed to resolve policies");
                        Vec::new()
                    }
                };
                let usage_min = usage_ms / 60000;
                let verdict = evaluate(&policies, usage_min);

                match verdict {
                    PolicyVerdict::Block {
                        policy_id, reason, ..
                    } => {
                        info!(%app_id, "Limit exceeded — enforcing block");
                        let entry = BlockedAppEntry {
                            app_id: app_id.as_ref().to_string(),
                            policy_id: policy_id.0 as u64,
                            reason: reason as u32,
                            blocked_since: now.timestamp_millis() as u64,
                        };
                        self.blocked_apps
                            .write()
                            .await
                            .entry(uid)
                            .or_default()
                            .insert(app_id.clone(), entry);
                        let _ = self.signal_tx.send(DaemonSignal::BlockedAppsChanged {
                            uid: uid.0,
                            app_id: app_id.clone(),
                            blocked: true,
                            reason: reason as u32,
                        });
                    }
                    PolicyVerdict::Notify { .. } => {
                        let body = format!("{} has exceeded its usage limit.", app_id);
                        if let Err(e) = self.platform.notify("Usage limit reached", &body).await {
                            tracing::warn!(%app_id, error = %e, "Failed to send notification");
                        }
                    }
                    PolicyVerdict::Ok => {}
                }
            }
        }
        Ok(())
    }

    /// Fetch policies for an app and filter by active schedule.
    async fn resolve_filtered_policies(
        &self,
        app_id: &AppId,
        categories: &[CategoryId],
        uid: Uid,
    ) -> anyhow::Result<Vec<PolicyConfig>> {
        let policies = self.repo.fetch_policies(app_id, categories, uid).await?;
        Ok(filter_policies_by_schedule(policies, self.clock.now()))
    }
}
