//! Blocking enforcement engine — orchestration layer.
//!
//! [`EnforcerActor`] receives [`PlatformEvent`]s, buffers them for batch
//! persistence, and evaluates policies from the database at minute-tick
//! boundaries. Persistence is delegated to [`BlockingRepo`] and overlay
//! state to [`OverlayManager`].

mod handlers;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::SystemTime;

use chrono::{DateTime, Utc};
use diesel_async::AsyncConnection;
use tokio::sync::mpsc;
use tracing::{error, info};
use wellbeing_core::*;

use crate::platform::{Platform, PlatformEvent};
use crate::policy::{PolicyConfig, PolicyVerdict, evaluate, filter_policies_by_schedule};
use crate::signal::DaemonSignal;
use crate::store::DbPool;

use super::buffer::EventBuffer;
use super::data::BlockingRepo;
use super::domain::InternalEvent;

/// Core enforcement actor, generic over [`Platform`] and [`Clock`].
pub struct EnforcerActor<P: Platform, C: Clock> {
    pub(crate) repo: BlockingRepo,
    platform: Arc<P>,
    pub(crate) blocked_apps:
        Arc<tokio::sync::RwLock<HashMap<Uid, HashMap<AppId, BlockedAppEntry>>>>,
    pub(crate) current_focus: HashMap<Uid, AppId>,
    /// Last known window title per uid, used to propagate title to
    /// synthetic events (e.g. extension-granted WindowFocused).
    pub(crate) last_titles: HashMap<Uid, WindowTitle>,
    pub(crate) clock: C,
    signal_tx: mpsc::UnboundedSender<DaemonSignal>,
    pub(crate) event_buffer: EventBuffer,
    internal_tx: mpsc::Sender<InternalEvent>,
}

impl<P: Platform, C: Clock> EnforcerActor<P, C> {
    pub fn new(
        pool: DbPool,
        platform: Arc<P>,
        clock: C,
        signal_tx: mpsc::UnboundedSender<DaemonSignal>,
        blocked_apps: Arc<tokio::sync::RwLock<HashMap<Uid, HashMap<AppId, BlockedAppEntry>>>>,
    ) -> (Self, mpsc::Receiver<InternalEvent>) {
        let (internal_tx, internal_rx) = mpsc::channel::<InternalEvent>(32);

        (
            Self {
                repo: BlockingRepo::new(pool),
                platform,
                blocked_apps,
                current_focus: HashMap::new(),
                last_titles: HashMap::new(),
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
                Some(InternalEvent::Flush(ack)) => {
                    if let Err(e) = self.flush_buffer().await {
                        error!(error = %e, "Timer-triggered flush failed");
                    } else {
                        self.prune_stale_blocks().await;
                        if let Err(e) = self.evaluate_and_enforce(self.clock.now()).await {
                            error!(error = %e, "Policy evaluation failed on minute-tick");
                        }
                    }
                    if let Some(tx) = ack {
                        let _ = tx.send(());
                    }
                }
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
        while let Ok(InternalEvent::Flush(ack)) = internal_rx.try_recv() {
            if let Err(e) = self.flush_buffer().await {
                error!(error = %e, "Final flush failed during shutdown");
            }
            if let Some(tx) = ack {
                let _ = tx.send(());
            }
        }
    }

    /// Flush buffered events to the database, apply closed-interval
    /// deltas from the buffer, and materialize open-interval deltas for
    /// all currently focused apps — all in a single transaction.
    pub async fn flush_buffer(&mut self) -> anyhow::Result<()> {
        let events = self.event_buffer.drain();
        let now = self.clock.now();
        let mut conn = self.repo.pool.get().await?;

        if !events.is_empty() {
            conn.transaction(async |conn| {
                // IMPORTANT: apply_closed_deltas_from_buffer MUST run BEFORE
                // flush_events. Its `else` branch (close event without matching
                // open in buffer) reads the last WindowFocused from the events
                // table to find the interval start. If flush_events inserted
                // new WindowFocused events first, the query may return a
                // *different* app's focus or an event with a *later* timestamp,
                // producing zero/negative duration and silently losing tracked
                // time.
                self.repo
                    .apply_closed_deltas_from_buffer(conn, &events, now)
                    .await?;
                self.repo.flush_events(conn, &events).await?;

                // increment_open_ms must run AFTER flush_events so it sees
                // the latest WindowFocused events in the DB. Running it
                // inside the transaction keeps the open/closed deltas atomic
                // and prevents re-adding time for intervals that were just
                // closed by apply_closed_deltas_from_buffer.
                for (&uid, app_id) in &self.current_focus {
                    self.repo
                        .increment_open_ms(conn, uid, app_id.clone(), now)
                        .await?;
                }

                Ok::<_, anyhow::Error>(())
            })
            .await?;
        } else {
            for (&uid, app_id) in &self.current_focus {
                self.repo
                    .increment_open_ms(&mut conn, uid, app_id.clone(), now)
                    .await?;
            }
        }

        // Use a set to avoid duplicate signals when multiple events share a UID.
        let mut seen_uids: std::collections::HashSet<u32> = std::collections::HashSet::new();
        for e in &events {
            seen_uids.insert(e.uid.0);
        }
        for uid in self.current_focus.keys() {
            seen_uids.insert(uid.0);
        }

        for uid_val in seen_uids {
            let _ = self
                .signal_tx
                .send(DaemonSignal::DailyUsageChanged { uid: uid_val });
        }
        Ok(())
    }

    /// Evaluate policies for all currently focused apps and enforce blocks.
    async fn evaluate_and_enforce(&mut self, now: DateTime<Utc>) -> anyhow::Result<()> {
        for (uid, app_id) in self.current_focus.clone() {
            if let Ok(usage_ms) = self.repo.fetch_usage(&app_id, uid, &self.clock).await {
                let categories = match self.repo.fetch_categories(&app_id, uid).await {
                    Ok(c) => c,
                    Err(e) => {
                        tracing::warn!(%app_id, error = %e, "Failed to fetch categories for policy evaluation");
                        Vec::new()
                    }
                };
                let policies = match self
                    .resolve_filtered_policies(&app_id, &categories, uid)
                    .await
                {
                    Ok(p) => p,
                    Err(e) => {
                        tracing::warn!(%app_id, error = %e, "Failed to resolve policies");
                        Vec::new()
                    }
                };
                // Convert milliseconds to minutes for policy evaluation
                // (policy limits are stored in minutes).
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
                            blocked_since: SystemTime::from(now)
                                .duration_since(SystemTime::UNIX_EPOCH)
                                .expect("blocked_since after epoch")
                                .as_millis() as u64,
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

    /// Prune blocked_apps entries whose app is no longer in current_focus.
    /// Runs on the minute-tick after flush/evaluate to keep block state
    /// consistent with the actual focused window.
    async fn prune_stale_blocks(&mut self) {
        let mut to_remove = Vec::new();
        for (uid, user_blocks) in self.blocked_apps.read().await.iter() {
            if let Some(focused) = self.current_focus.get(uid) {
                for app_id in user_blocks.keys() {
                    if app_id != focused {
                        to_remove.push((*uid, app_id.clone()));
                    }
                }
            } else {
                for app_id in user_blocks.keys() {
                    to_remove.push((*uid, app_id.clone()));
                }
            }
        }
        for (uid, app_id) in to_remove {
            if let Some(user_blocks) = self.blocked_apps.write().await.get_mut(&uid)
                && user_blocks.remove(&app_id).is_some()
            {
                let _ = self.signal_tx.send(DaemonSignal::BlockedAppsChanged {
                    uid: uid.0,
                    app_id: app_id.clone(),
                    blocked: false,
                    reason: 0,
                });
            }
        }
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
