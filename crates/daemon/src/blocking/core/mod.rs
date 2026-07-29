//! Blocking enforcement engine — orchestration layer.
//!
//! [`EnforcerActor`] receives [`PlatformEvent`]s from the plugin, buffers
//! them for batch persistence, and evaluates policies from the database.
//!
//! - Focus is **always written first**. The event log is true
//!   append-only: Focus always written, Blocked(event_type=8) terminates.
//! - Plugin decides event tag: Focus(tag=0) if app not in BlockedApps,
//!   Block(tag=2) if it is. Daemon writes whatever the plugin sends.
//! - `evaluate_and_enforce` uses the new `evaluate()` function which is
//!   priority-ordered, first-match-wins.
//! - Per-minute tick re-evaluates the single focused app using same `evaluate()`.

mod buffer;
mod handlers;

pub(crate) use buffer::EventBuffer;

use std::collections::HashMap;
use std::sync::Arc;

use chrono::{Datelike, Timelike};
use futures::stream::{FuturesUnordered, StreamExt};
use tokio::sync::mpsc;
use tokio::sync::oneshot;
use tracing::{debug, error, info};

use wellbeing_core::*;

use super::data::BlockingRepo;
use crate::platform::linux::{ManagerClient, PluginRegistry};
use crate::platform::{Platform, PlatformEvent, PowerEventKind};
use crate::policy::Policy;
use crate::policy::data::PolicyRepo;
use crate::policy::evaluate;
use crate::signal::DaemonSignal;
use crate::store::DbPool;

pub enum InternalEvent {
    /// Flush the event buffer. The optional oneshot sender is signaled
    /// after the flush completes.
    Flush(Option<oneshot::Sender<()>>),
    /// Insert a shutdown event for every registered UID, then flush
    /// the event buffer. Used when the daemon receives SIGTERM/SIGINT
    /// so the shutdown is recorded before exit.
    Shutdown(Option<oneshot::Sender<()>>),
    /// A policy was mutated for the given user. Re-evaluate the currently
    /// focused app and update the blocked-apps map accordingly.
    PolicyMutated { owner_id: Uid },
}

pub struct EnforcerActor<P: Platform, C: Clock> {
    pub(crate) blocking_repo: BlockingRepo,
    pub(crate) policy_repo: PolicyRepo,
    pub(crate) registry: Arc<tokio::sync::RwLock<PluginRegistry>>,
    pub(crate) blocked_apps:
        Arc<tokio::sync::RwLock<HashMap<Uid, HashMap<AppClass, BlockedAppEntry>>>>,
    platform: Arc<P>,
    pub(crate) clock: C,
    signal_tx: mpsc::UnboundedSender<DaemonSignal>,
    pub(crate) event_buffer: EventBuffer,
    internal_tx: mpsc::Sender<InternalEvent>,
    last_midnight_reset: Option<chrono::NaiveDate>,
    /// Per-tick app-id cache shared across delta processing and policy
    /// evaluation, eliminating redundant `SELECT id FROM apps` round-trips
    /// for `AppClass` values that were already ensured during the flush phase.
    app_cache: tokio::sync::Mutex<HashMap<AppClass, i32>>,
}

impl<P: Platform, C: Clock> EnforcerActor<P, C> {
    pub fn new(
        pool: DbPool,
        platform: Arc<P>,
        registry: Arc<tokio::sync::RwLock<PluginRegistry>>,
        clock: C,
        signal_tx: mpsc::UnboundedSender<DaemonSignal>,
        blocked_apps: Arc<tokio::sync::RwLock<HashMap<Uid, HashMap<AppClass, BlockedAppEntry>>>>,
    ) -> (Self, mpsc::Receiver<InternalEvent>) {
        let (internal_tx, internal_rx) = mpsc::channel::<InternalEvent>(32);

        (
            Self {
                policy_repo: PolicyRepo::new(pool.clone()),
                blocking_repo: BlockingRepo::new(pool),
                platform,
                registry,
                blocked_apps,
                clock,
                signal_tx,
                event_buffer: EventBuffer::default(),
                internal_tx,
                last_midnight_reset: None,
                app_cache: tokio::sync::Mutex::new(HashMap::new()),
            },
            internal_rx,
        )
    }

    pub fn flush_handle(&self) -> mpsc::Sender<InternalEvent> {
        self.internal_tx.clone()
    }

    pub fn policy_mutation_handle(&self) -> mpsc::Sender<InternalEvent> {
        self.internal_tx.clone()
    }

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
                if let Err(e) = self.maybe_midnight_reset().await {
                    error!(error = %e, "midnight_reset failed");
                }
                if let Some(tx) = ack {
                    let _ = tx.send(());
                }
            }
            InternalEvent::Shutdown(ack) => {
                let uids = self.registry.read().await.registered_uids();
                for &uid in &uids {
                    self.event_buffer.push(
                        PlatformEvent::PowerEvent {
                            kind: PowerEventKind::Shutdown,
                            uid,
                        },
                        self.clock.now(),
                    );
                }
                // Flush so the shutdown events are persisted immediately.
                if let Err(e) = self.flush_buffer().await {
                    error!(error = %e, "Shutdown flush failed");
                }
                if let Some(tx) = ack {
                    let _ = tx.send(());
                }
            }
            InternalEvent::PolicyMutated { owner_id } => {
                if let Err(e) = self
                    .handle_policy_mutation(owner_id, self.clock.now())
                    .await
                {
                    error!(error = %e, "Policy mutation handler failed");
                }
            }
        }
    }

    pub async fn flush_buffer(&mut self) -> anyhow::Result<()> {
        let now = self.clock.now();
        let all_uids = {
            let mut set: std::collections::HashSet<Uid> =
                self.event_buffer.uids().into_iter().collect();
            set.extend(self.registry.read().await.registered_uids());
            set.into_iter().collect::<Vec<_>>()
        };

        let events = self.event_buffer.drain();
        {
            let mut cache = self.app_cache.lock().await;
            self.blocking_repo
                .flush_with_deltas(&events, &all_uids, now, &mut cache)
                .await?;
        }

        self.emit_daily_usage_changed(&all_uids);
        Ok(())
    }

    fn emit_daily_usage_changed(&self, uids: &[Uid]) {
        for &uid in uids {
            let _ = self.signal_tx.send(DaemonSignal::DailyUsageChanged { uid });
        }
    }

    async fn evaluate_and_enforce(&self, now: chrono::DateTime<chrono::Utc>) -> anyhow::Result<()> {
        let uids = self.registry.read().await.registered_uids();

        // Parallel per-UID evaluation so plugin RTTs overlap.
        let mut futures: FuturesUnordered<_> = uids
            .into_iter()
            .map(|uid| self.evaluate_single_uid(uid, now))
            .collect();

        while let Some(result) = futures.next().await {
            result?;
        }
        Ok(())
    }

    /// Query one UID's current focus and enforce policies. Used by
    /// [`evaluate_and_enforce`] to parallelise per-UID plugin RTTs.
    async fn evaluate_single_uid(
        &self,
        uid: Uid,
        now: chrono::DateTime<chrono::Utc>,
    ) -> anyhow::Result<()> {
        let proxy = {
            let reg = self.registry.read().await;
            reg.focus_proxy_for_uid(uid)
        };

        let Some((uid, proxy)) = proxy else {
            debug!(
                ?uid,
                "evaluate_and_enforce: no plugin registered — skipping"
            );
            return Ok(());
        };

        let client = ManagerClient::new(uid, proxy);
        let focused = client.current_focus().await;

        let Some(event) = focused else {
            debug!(
                ?uid,
                "evaluate_and_enforce: plugin unreachable — cleaning up stale registration"
            );
            self.registry.write().await.unregister_by_uid(uid);
            return Ok(());
        };
        let Some(app_class) = event.app_class() else {
            debug!(
                ?uid,
                "evaluate_and_enforce: current focus has no app_class — skipping"
            );
            return Ok(());
        };

        debug!(?uid, %app_class, "evaluate_and_enforce: evaluating focused app");

        self.evaluate_and_apply(uid, app_class, now).await
    }

    /// Evaluate a specific app for a uid and apply the result to blocked_apps.
    /// Emits BlockedAppsChanged signals as needed.
    /// Used by handle_event (with event payload data) and evaluate_and_enforce
    /// (with re-queried GetFocusState data).
    async fn evaluate_and_apply(
        &self,
        uid: Uid,
        app_class: &AppClass,
        now: chrono::DateTime<chrono::Utc>,
    ) -> anyhow::Result<()> {
        match self.evaluate_single_app(uid, app_class, now).await? {
            Some(entry) => {
                let reason = entry.reason;
                let was_new = self
                    .blocked_apps
                    .write()
                    .await
                    .entry(uid)
                    .or_default()
                    .insert(app_class.clone(), entry)
                    .is_none();
                if was_new {
                    info!(
                        ?uid, %app_class, ?reason,
                        "evaluate_and_apply: NEW block — emitting BlockedAppsChanged {{blocked: true}}"
                    );
                    let _ = self.signal_tx.send(DaemonSignal::BlockedAppsChanged {
                        uid,
                        app_class: app_class.clone(),
                        blocked: true,
                        reason,
                    });
                } else {
                    debug!(
                        ?uid, %app_class,
                        "evaluate_and_apply: app was already blocked — no signal"
                    );
                }
            }
            None => {
                if self
                    .blocked_apps
                    .write()
                    .await
                    .entry(uid)
                    .or_default()
                    .remove(app_class)
                    .is_some()
                {
                    info!(
                        ?uid, %app_class,
                        "evaluate_and_apply: unblocked — emitting BlockedAppsChanged {{blocked: false}}"
                    );
                    let _ = self.signal_tx.send(DaemonSignal::BlockedAppsChanged {
                        uid,
                        app_class: app_class.clone(),
                        blocked: false,
                        reason: BlockReason::AppBlock,
                    });
                }
            }
        }
        Ok(())
    }

    async fn evaluate_single_app(
        &self,
        uid: Uid,
        app_class: &AppClass,
        now: chrono::DateTime<chrono::Utc>,
    ) -> anyhow::Result<Option<BlockedAppEntry>> {
        let app_id = {
            let cache = self.app_cache.lock().await;
            if let Some(&id) = cache.get(app_class) {
                id
            } else {
                // Cache miss — drop lock to avoid holding across ensure_app await.
                drop(cache);
                match self.blocking_repo.ensure_app(app_class).await {
                    Ok(id) => {
                        self.app_cache.lock().await.insert(app_class.clone(), id);
                        id
                    }
                    Err(e) => {
                        tracing::warn!(%app_class, error = %e, "Failed to upsert app");
                        return Ok(None);
                    }
                }
            }
        };

        let category_discriminants = match self
            .blocking_repo
            .resolve_category_discriminants(app_class, uid)
            .await
        {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(%app_class, error = %e, "Failed to fetch category discriminants");
                Vec::new()
            }
        };

        // Use actual wall-clock time for schedule resolution (day_mask, minute_of_day)
        // to avoid mocked-clock discrepancies in minute-granularity checks.
        let now_local = chrono::Utc::now();
        let minute_of_day = (now_local.hour() * 60 + now_local.minute()) as i32;
        let day_mask = 1i32 << now_local.weekday().num_days_from_sunday();
        let policies: Vec<Policy> = match self
            .policy_repo
            .resolve_for_app(
                app_id,
                &category_discriminants,
                uid,
                day_mask,
                minute_of_day,
            )
            .await
        {
            Ok(p) => {
                debug!(
                    %app_class, app_id, policy_count = p.len(),
                    day_mask, minute_of_day,
                    "evaluate_single_app: fetched policies"
                );
                p
            }
            Err(e) => {
                tracing::warn!(%app_class, error = %e, "Failed to fetch policies");
                Vec::new()
            }
        };

        if policies.is_empty() {
            debug!(%app_class, app_id, "evaluate_single_app: no matching policies — unrestricted");
            return Ok(None);
        }

        let today = now.format("%Y-%m-%d").to_string();
        let usage_ms = match self
            .blocking_repo
            .get_today_usage(app_id, uid, &today)
            .await
        {
            Ok(u) => u,
            Err(e) => {
                tracing::warn!(%app_class, error = %e, "Failed to fetch usage");
                0
            }
        };
        let usage_min = (usage_ms / 60000) as u64;
        let result = evaluate(&policies, usage_min, now);

        debug!(
            %app_class,
            terminating = ?result.terminating,
            notifies = result.notifies.len(),
            "evaluate_single_app: evaluate() returned"
        );

        // Handle Notify effects (non-terminating) — fire-and-forget.
        for (_notify_id, _effect) in &result.notifies {
            let body = format!("{} has exceeded its usage threshold.", app_class);
            if let Err(e) = self.platform.notify("Usage limit reached", &body).await {
                tracing::warn!(%app_class, error = %e, "Failed to send notification");
            }
        }

        // Handle terminating policy — only Block / TimeLimit produce an entry.
        if let Some((policy_id, ref effect)) = result.terminating {
            match effect {
                crate::policy::Effect::Block => {
                    return Ok(Some(BlockedAppEntry {
                        app_class: app_class.clone(),
                        policy_id,
                        reason: BlockReason::AppBlock,
                        blocked_since: now.timestamp_millis() as u64,
                    }));
                }
                crate::policy::Effect::TimeLimit { .. } => {
                    return Ok(Some(BlockedAppEntry {
                        app_class: app_class.clone(),
                        policy_id,
                        reason: BlockReason::AppTimeLimit,
                        blocked_since: now.timestamp_millis() as u64,
                    }));
                }
                crate::policy::Effect::Allow | crate::policy::Effect::Notify { .. } => {
                    // Allow = no-op; Notify already handled above.
                }
            }
        }
        Ok(None)
    }

    async fn handle_policy_mutation(
        &mut self,
        owner_id: Uid,
        now: chrono::DateTime<chrono::Utc>,
    ) -> anyhow::Result<()> {
        let proxy = {
            let reg = self.registry.read().await;
            reg.focus_proxy_for_uid(owner_id)
        };
        let Some((uid, proxy)) = proxy else {
            return Ok(());
        };
        let client = ManagerClient::new(uid, proxy);
        let focused = client.current_focus().await;
        let Some(event) = focused else {
            self.registry.write().await.unregister_by_uid(owner_id);
            return Ok(());
        };
        let Some(app_class) = event.app_class() else {
            return Ok(());
        };

        match self.evaluate_single_app(owner_id, app_class, now).await? {
            Some(entry) => {
                let reason = entry.reason;
                let was_new = self
                    .blocked_apps
                    .write()
                    .await
                    .entry(owner_id)
                    .or_default()
                    .insert(app_class.clone(), entry)
                    .is_none();
                if was_new {
                    let _ = self.signal_tx.send(DaemonSignal::BlockedAppsChanged {
                        uid: owner_id,
                        app_class: app_class.clone(),
                        blocked: true,
                        reason,
                    });
                }
            }
            None => {
                if self
                    .blocked_apps
                    .write()
                    .await
                    .entry(owner_id)
                    .or_default()
                    .remove(app_class)
                    .is_some()
                {
                    let _ = self.signal_tx.send(DaemonSignal::BlockedAppsChanged {
                        uid: owner_id,
                        app_class: app_class.clone(),
                        blocked: false,
                        reason: BlockReason::AppBlock,
                    });
                }
            }
        }
        Ok(())
    }

    async fn maybe_midnight_reset(&mut self) -> anyhow::Result<()> {
        let now_utc = chrono::Utc::now();
        let today = now_utc.date_naive();
        if let Some(last) = self.last_midnight_reset
            && last >= today
        {
            return Ok(());
        }
        // Only perform the reset if we have a prior record (i.e. this is not
        // the very first check after actor start).
        if self.last_midnight_reset.is_some() {
            self.midnight_reset().await?;
        }
        self.last_midnight_reset = Some(today);
        Ok(())
    }

    /// Remove all time-limit-based blocks (daily resets) and emit
    /// `BlockedAppsChanged` with `blocked: false` for each removed entry.
    ///
    /// Hard blocks (`AppBlock`, `CategoryBlock`) are permanent and survive
    /// midnight rollover.
    async fn midnight_reset(&mut self) -> anyhow::Result<()> {
        let mut blocks = self.blocked_apps.write().await;
        for (uid, apps) in blocks.iter_mut() {
            apps.retain(|_app_class, entry| {
                if matches!(
                    entry.reason,
                    BlockReason::AppBlock | BlockReason::CategoryBlock
                ) {
                    true
                } else {
                    // Time-limit blocks — daily, reset.
                    let _ = self.signal_tx.send(DaemonSignal::BlockedAppsChanged {
                        uid: *uid,
                        app_class: entry.app_class.clone(),
                        blocked: false,
                        reason: entry.reason,
                    });
                    false
                }
            });
        }
        Ok(())
    }
}
