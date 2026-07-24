//! Blocking enforcement engine — orchestration layer.
//!
//! [`EnforcerActor`] receives [`PlatformEvent`]s, buffers them for batch
//! persistence, and evaluates policies from the database at minute-tick
//! boundaries. Persistence is delegated to [`BlockingRepo`] and overlay
//! state to [`OverlayManager`].

use std::collections::HashMap;
use std::sync::Arc;
use std::time::SystemTime;

use chrono::{DateTime, Utc};
use diesel::{ExpressionMethods, QueryDsl, SelectableHelper};
use diesel_async::{AsyncConnection, RunQueryDsl};
use tokio::sync::mpsc;
use tracing::{error, info, warn};
use wellbeing_core::*;

use crate::platform::{Platform, PlatformEvent};
use crate::policy::{PolicyConfig, PolicyVerdict, evaluate, filter_policies_by_schedule};
use crate::signal::DaemonSignal;
use crate::store::DbPool;

use super::buffer::{BufferedEvent, EventBuffer};
use super::data::{BlockingRepo, EventRow};
use super::domain::*;
use super::overlay::OverlayManager;
use wellbeing_core::event_types::{CLOSE_EVENT_TYPES, EVENT_IDLE, EVENT_RESUMED};

/// Core enforcement actor, generic over [`Platform`] and [`Clock`].
pub struct EnforcerActor<P: Platform, C: Clock> {
    pub(crate) repo: BlockingRepo,
    platform: Arc<P>,
    overlay: OverlayManager,
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
        active_blocks: Arc<tokio::sync::RwLock<HashMap<Uid, HashMap<AppId, ActiveBlockEntry>>>>,
    ) -> (Self, mpsc::Receiver<InternalEvent>) {
        let (internal_tx, internal_rx) = mpsc::channel::<InternalEvent>(32);

        (
            Self {
                repo: BlockingRepo::new(pool),
                platform,
                overlay: OverlayManager::new(active_blocks, signal_tx.clone()),
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

    /// Recover from crash / startup / system resume by rebuilding routing state
    /// from today's event sequence and buffering any stale open intervals.
    pub async fn recover(&mut self) -> anyhow::Result<()> {
        let now = self.clock.now();
        let events = self.recover_read_events().await?;
        self.recover_rebuild_focus(&events);
        self.recover_buffer_stale(&events, now);

        let mut seen_uids: std::collections::HashSet<u32> = std::collections::HashSet::new();
        for e in &events {
            seen_uids.insert(e.user_id as u32);
        }
        for uid_val in &seen_uids {
            let _ = self
                .signal_tx
                .send(DaemonSignal::DailyUsageChanged { uid: *uid_val });
        }

        Ok(())
    }

    /// Query today's events from the database in chronological order.
    async fn recover_read_events(&self) -> anyhow::Result<Vec<EventRow>> {
        let now = self.clock.now();
        let day_start = now
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp_millis();
        let day_end = now
            .date_naive()
            .and_hms_opt(23, 59, 59)
            .unwrap()
            .and_utc()
            .timestamp_millis();

        let mut conn = self.repo.pool().get().await?;
        let events: Vec<EventRow> = crate::store::schema::events::table
            .filter(crate::store::schema::events::timestamp.ge(day_start))
            .filter(crate::store::schema::events::timestamp.le(day_end))
            .order(crate::store::schema::events::timestamp.asc())
            .select(EventRow::as_select())
            .load(&mut conn)
            .await?;
        drop(conn);
        Ok(events)
    }

    /// Rebuild `current_focus` from the event sequence — WindowFocused
    /// starts an interval, close events end it.
    fn recover_rebuild_focus(&mut self, events: &[EventRow]) {
        self.current_focus.clear();
        for event in events {
            match event.event_type {
                0 => {
                    if let Some(app_id_str) = &event.app_id
                        && let Ok(aid) = AppId::new(app_id_str)
                    {
                        self.current_focus.insert(Uid(event.user_id as u32), aid);
                    }
                }
                t if CLOSE_EVENT_TYPES.contains(&t) => {
                    self.current_focus.remove(&Uid(event.user_id as u32));
                }
                _ => {}
            }
        }
    }

    /// If a uid still has focus in current_focus, the last event was
    /// WindowFocused with no close — buffer a synthetic Unfocused so the
    /// open interval gets closed at the next flush.
    fn recover_buffer_stale(&mut self, _events: &[EventRow], now: DateTime<Utc>) {
        let stale_uids: Vec<(Uid, AppId)> = self
            .current_focus
            .iter()
            .map(|(uid, app_id)| (*uid, app_id.clone()))
            .collect();
        for (uid, app_id) in &stale_uids {
            self.event_buffer.push(BufferedEvent {
                uid: *uid,
                app_id: app_id.clone(),
                event_type: 1, // EVENT_UNFOCUSED
                timestamp: now,
                title: None,
            });
        }

        self.current_focus.clear();
    }

    /// Returns `true` when the in-memory focus map is empty (no open intervals).
    /// Used by the session‑reconciliation logic in `main.rs` to decide whether
    /// to inject synthetic close events after boot, unlock, or resume.
    pub fn is_current_focus_empty(&self) -> bool {
        self.current_focus.is_empty()
    }

    /// Returns a snapshot of the current in-memory focus map.
    /// Used by the session‑reconciliation logic in `main.rs`.
    pub fn current_focus_snapshot(&self) -> HashMap<Uid, AppId> {
        self.current_focus.clone()
    }

    pub async fn handle_event(&mut self, event: PlatformEvent) {
        match event {
            PlatformEvent::WindowFocused {
                app_id,
                uid,
                overlay_shown,
                title,
                ..
            } => {
                self.handle_window_focused(app_id, uid, title, overlay_shown)
                    .await;
            }
            PlatformEvent::Unfocused => self.handle_unfocused().await,
            PlatformEvent::IdleActivity => self.handle_idle_activity().await,
            PlatformEvent::ResumedActivity => self.handle_resumed_activity().await,
            PlatformEvent::ResumedSystem => {
                // System resumed from sleep — focus events will arrive
                // naturally from the compositor; no action needed.
            }
            PlatformEvent::Slept | PlatformEvent::Locked | PlatformEvent::LoggedOut => {
                self.handle_session_event(event).await;
            }
            PlatformEvent::ShutDown => self.handle_shut_down().await,
            PlatformEvent::UserAction {
                app_id,
                action,
                uid,
            } => self.handle_user_action(app_id, action, uid).await,
        }

        // Buffer threshold flush
        if self.event_buffer.len() >= 100
            && let Err(e) = self.flush_buffer().await
        {
            error!(error = %e, "Count-triggered flush failed");
        }
    }

    /// Handle a window focus switch: dedup, unfocus previous, buffer events.
    async fn handle_window_focused(
        &mut self,
        app_id: AppId,
        uid: Uid,
        title: WindowTitle,
        _overlay_shown: bool,
    ) {
        if matches!(self.current_focus.get(&uid), Some(prev) if *prev == app_id) {
            // Update the last known title even when the app hasn't changed,
            // so synthetic events (extension grants) get the current title.
            self.last_titles.insert(uid, title);
            return;
        }
        if let Some(prev) = self.current_focus.insert(uid, app_id.clone()) {
            self.event_buffer.push(BufferedEvent {
                uid,
                app_id: prev,
                event_type: 1, // EVENT_UNFOCUSED
                timestamp: self.clock.now(),
                title: None,
            });
        }
        self.last_titles.insert(uid, title.clone());
        self.event_buffer.push(BufferedEvent {
            uid,
            app_id,
            event_type: 0, // EVENT_WINDOW_FOCUSED
            timestamp: self.clock.now(),
            title: Some(title),
        });
    }

    /// Push a [`BufferedEvent`] for each entry in `current_focus` with the given
    /// `event_type`. If `drain` is true, the focus map is cleared after buffering.
    fn buffer_event_for_all_focused(&mut self, event_type: i32, drain: bool) {
        if drain {
            for (uid, app_id) in self.current_focus.drain() {
                self.event_buffer.push(BufferedEvent {
                    uid,
                    app_id,
                    event_type,
                    timestamp: self.clock.now(),
                    title: None,
                });
            }
        } else {
            for (uid, app_id) in &self.current_focus {
                self.event_buffer.push(BufferedEvent {
                    uid: *uid,
                    app_id: app_id.clone(),
                    event_type,
                    timestamp: self.clock.now(),
                    title: None,
                });
            }
        }
    }

    /// Handle global unfocused: drain all current focus entries.
    async fn handle_unfocused(&mut self) {
        self.buffer_event_for_all_focused(1, true);
    }

    /// Handle idle transition: buffer idle for all focused apps.
    async fn handle_idle_activity(&mut self) {
        self.buffer_event_for_all_focused(EVENT_IDLE, false);
    }

    /// Handle resume from idle: buffer resumed for all focused apps.
    async fn handle_resumed_activity(&mut self) {
        self.buffer_event_for_all_focused(EVENT_RESUMED, false);
    }

    /// Handle session events (sleep, lock, logout): drain focus with
    /// the appropriate event type.
    async fn handle_session_event(&mut self, event: PlatformEvent) {
        let event_type = match event {
            PlatformEvent::Slept => 4,
            PlatformEvent::Locked => 6,
            PlatformEvent::LoggedOut => 7,
            _ => unreachable!(),
        };
        self.buffer_event_for_all_focused(event_type, true);
    }

    /// Handle shut down: drain focus with shut-down event type.
    async fn handle_shut_down(&mut self) {
        self.buffer_event_for_all_focused(5, true);
    }

    /// Current blocking state (delegated to overlay manager).
    pub async fn blocking_state(&self) -> BlockingState {
        self.overlay.blocking_state().await
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
                            // Platform event channel closed — process any
                            // remaining internal events before exiting.
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

    pub async fn handle_user_action(&mut self, app_id: AppId, action: u32, uid: Uid) {
        let policy_id = self.overlay.lookup_policy_id(uid, &app_id).await;

        if policy_id.0 == 0 {
            return;
        }

        match action {
            0 => {
                let now = self.clock.now();
                let today = now.format("%Y-%m-%d").to_string();

                let title_opt = self.last_titles.get(&uid).map(|t| t.as_str());

                if let Err(e) = self
                    .repo
                    .write_event(
                        0,
                        uid.0 as i32,
                        now.timestamp_millis(),
                        Some(app_id.as_ref()),
                        title_opt,
                    )
                    .await
                {
                    error!(%app_id, error = %e, "Failed to write synthetic WindowFocused");
                    return;
                }

                if let Err(e) = self
                    .repo
                    .mark_daily_usage_extended(&app_id, &today, uid)
                    .await
                {
                    error!(%app_id, error = %e, "Failed to mark daily_usage extended");
                    return;
                }

                self.current_focus.insert(uid, app_id.clone());

                self.overlay.unblock(&app_id, uid).await;
                info!(%app_id, "Extension granted");
            }
            1 => {
                self.overlay.unblock(&app_id, uid).await;
            }
            _ => {
                warn!(action, "Unknown overlay action value");
            }
        }
    }

    /// Flush buffered events to the database, apply closed-interval
    /// deltas from the buffer, and materialize open-interval deltas for
    /// all currently focused apps — all in a single transaction.
    pub async fn flush_buffer(&mut self) -> anyhow::Result<()> {
        let events = self.event_buffer.drain();
        let now = self.clock.now();
        let mut conn = self.repo.pool().get().await?;

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
                Ok::<_, anyhow::Error>(())
            })
            .await?;
        }

        for (&uid, app_id) in &self.current_focus {
            self.repo
                .increment_open_ms(&mut conn, uid, app_id.clone(), now)
                .await?;
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
                let categories = self
                    .repo
                    .fetch_categories(&app_id, uid)
                    .await
                    .unwrap_or_default();
                let policies = self
                    .resolve_filtered_policies(&app_id, &categories, uid)
                    .await
                    .unwrap_or_default();
                // Convert milliseconds to minutes for policy evaluation
                // (policy limits are stored in minutes).
                let usage_min = (usage_ms.0 / 60000, usage_ms.1);
                let verdict = evaluate(&policies, usage_min.0, usage_min.1);

                match verdict {
                    PolicyVerdict::Block {
                        policy_id, reason, ..
                    } => {
                        info!(%app_id, "Limit exceeded — enforcing block");
                        let actions = self
                            .overlay
                            .determine_actions(policy_id, &policies, usage_min);
                        self.overlay
                            .show_block(
                                &app_id,
                                uid,
                                policy_id,
                                reason,
                                actions,
                                SystemTime::from(now),
                            )
                            .await;
                    }
                    PolicyVerdict::Notify { .. } => {
                        let body = format!("{} has exceeded its usage limit.", app_id);
                        if let Err(e) = self.platform.notify("Usage limit reached", &body).await {
                            warn!(%app_id, error = %e, "Failed to send notification");
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blocking::test_support::{MockPlatform, app, dt, setup};
    use crate::store::StoreBuilder;

    use diesel::{ExpressionMethods, QueryDsl};
    use diesel_async::RunQueryDsl;
    use tokio::sync::RwLock;

    #[tokio::test]
    async fn test_flush_emits_signal() {
        let (_tmp, _pool, mut actor, mut signal_rx) = setup().await;

        actor.event_buffer.push(BufferedEvent {
            uid: Uid(1000),
            app_id: app("firefox"),
            event_type: 0, // EVENT_WINDOW_FOCUSED
            timestamp: dt(1_000_000),
            title: None,
        });
        actor.event_buffer.push(BufferedEvent {
            uid: Uid(1000),
            app_id: app("firefox"),
            event_type: 1, // EVENT_UNFOCUSED
            timestamp: dt(1_000_100),
            title: None,
        });

        actor.flush_buffer().await.expect("flush should succeed");

        // DailyUsageChanged emitted for uid in buffer
        match signal_rx.try_recv() {
            Ok(DaemonSignal::DailyUsageChanged { uid }) => assert_eq!(uid, 1000),
            other => panic!("Expected DailyUsageChanged, got {:?}", other),
        }
        assert!(signal_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn test_empty_flush_no_signal() {
        let (_tmp, _pool, mut actor, mut signal_rx) = setup().await;

        let result = actor.flush_buffer().await;

        assert!(result.is_ok(), "empty flush should succeed");
        assert!(signal_rx.try_recv().is_err(), "no signal on empty flush");
    }

    #[tokio::test]
    async fn test_count_trigger_flushes_at_100_events() {
        let (_tmp, _pool, mut actor, _signal_rx) = setup().await;

        for i in 0..99 {
            let app_id = AppId::new(&format!("test.app.{i}")).unwrap();
            actor
                .handle_event(PlatformEvent::WindowFocused {
                    app_id,
                    title: WindowTitle::new("test"),
                    pid: Pid(1234),
                    uid: Uid(i + 1),
                    overlay_shown: false,
                })
                .await;
        }
        assert_eq!(actor.event_buffer.len(), 99);

        actor
            .handle_event(PlatformEvent::WindowFocused {
                app_id: AppId::new("test.app.100").unwrap(),
                title: WindowTitle::new("test"),
                pid: Pid(1234),
                uid: Uid(1000),
                overlay_shown: false,
            })
            .await;

        assert!(actor.event_buffer.is_empty());
    }

    #[tokio::test]
    async fn test_recover_emits_daily_usage_changed() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let pool = StoreBuilder::new(tmp.path().to_path_buf())
            .build()
            .await
            .unwrap();
        let platform = Arc::new(MockPlatform);
        let (signal_tx, mut signal_rx) = mpsc::unbounded_channel();
        let active_blocks = Arc::new(RwLock::new(HashMap::new()));
        let (mut enforcer, _) = EnforcerActor::new(
            pool.clone(),
            platform,
            SystemClock,
            signal_tx,
            active_blocks,
        );

        let uid = Uid(1000);
        let now = SystemClock.now();
        let today_date = now.format("%Y-%m-%d").to_string();
        let ts_millis = chrono::NaiveDateTime::parse_from_str(
            &format!("{} 10:00:00", today_date),
            "%Y-%m-%d %H:%M:%S",
        )
        .unwrap()
        .and_utc()
        .timestamp_millis();
        enforcer
            .repo
            .write_event(0, uid.0 as i32, ts_millis, Some("firefox"), None)
            .await
            .unwrap();

        enforcer.recover().await.unwrap();

        match signal_rx.try_recv() {
            Ok(DaemonSignal::DailyUsageChanged { uid: u }) => assert_eq!(u, 1000),
            other => panic!("Expected DailyUsageChanged, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_grant_extension_writes_immediately() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let pool = StoreBuilder::new(tmp.path().to_path_buf())
            .build()
            .await
            .unwrap();
        let platform = Arc::new(MockPlatform);
        let (signal_tx, _) = mpsc::unbounded_channel();
        let uid = Uid(1000);
        let app_id = AppId::new("test.app").unwrap();
        let active_blocks = Arc::new(RwLock::new(
            std::iter::once((
                uid,
                std::iter::once((
                    app_id.clone(),
                    ActiveBlockEntry {
                        app_id: app_id.as_ref().to_string(),
                        policy_id: 42,
                        reason: 0,
                        blocked_since: SystemTime::now()
                            .duration_since(SystemTime::UNIX_EPOCH)
                            .unwrap()
                            .as_millis() as u64,
                        available_actions: vec![0],
                    },
                ))
                .collect::<HashMap<_, _>>(),
            ))
            .collect::<HashMap<_, _>>(),
        ));
        let (mut enforcer, _) = EnforcerActor::new(
            pool.clone(),
            platform,
            SystemClock,
            signal_tx,
            active_blocks,
        );

        enforcer.current_focus.insert(uid, app_id.clone());

        enforcer.handle_user_action(app_id.clone(), 0, uid).await;

        let mut conn = pool.get().await.unwrap();

        let event_count: i64 = crate::store::schema::events::table
            .count()
            .get_result(&mut conn)
            .await
            .unwrap();
        assert_eq!(event_count, 1);

        let today = enforcer.clock.now().format("%Y-%m-%d").to_string();
        let extended_flag: bool = crate::store::schema::daily_usage::table
            .select(crate::store::schema::daily_usage::extended)
            .filter(crate::store::schema::daily_usage::date.eq(&today))
            .filter(crate::store::schema::daily_usage::user_id.eq(uid.0 as i32))
            .filter(crate::store::schema::daily_usage::app_id.eq(app_id.as_ref()))
            .first(&mut conn)
            .await
            .unwrap();
        assert!(extended_flag);

        assert!(enforcer.event_buffer.is_empty());
    }

    #[tokio::test]
    async fn test_daily_usage_equivalence() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let pool = StoreBuilder::new(tmp.path().to_path_buf())
            .build()
            .await
            .unwrap();
        let platform = Arc::new(MockPlatform);
        let (signal_tx, _) = mpsc::unbounded_channel();
        let active_blocks = Arc::new(RwLock::new(HashMap::new()));

        let start = Utc::now();
        let inner_clock = Arc::new(std::sync::Mutex::new(VirtualClock::new(start)));
        let test_clock = inner_clock.clone();

        struct SharedClock(Arc<std::sync::Mutex<VirtualClock>>);

        impl Clock for SharedClock {
            fn now(&self) -> DateTime<Utc> {
                self.0.lock().unwrap().now()
            }
        }

        let shared = SharedClock(inner_clock);
        let (mut enforcer, _) =
            EnforcerActor::new(pool.clone(), platform, shared, signal_tx, active_blocks);

        let uid = Uid(1000);

        enforcer
            .handle_event(PlatformEvent::WindowFocused {
                app_id: AppId::new("firefox").unwrap(),
                title: WindowTitle::new("Mozilla Firefox"),
                pid: Pid(1234),
                uid,
                overlay_shown: false,
            })
            .await;

        test_clock
            .lock()
            .unwrap()
            .advance(chrono::Duration::minutes(10));

        enforcer
            .handle_event(PlatformEvent::WindowFocused {
                app_id: AppId::new("code").unwrap(),
                title: WindowTitle::new("Visual Studio Code"),
                pid: Pid(5678),
                uid,
                overlay_shown: false,
            })
            .await;

        test_clock
            .lock()
            .unwrap()
            .advance(chrono::Duration::minutes(5));

        enforcer.handle_event(PlatformEvent::Unfocused).await;
        enforcer.flush_buffer().await.unwrap();

        let mut conn = pool.get().await.unwrap();

        let rows: Vec<(String, i32, String, i32, i32, bool)> =
            crate::store::schema::daily_usage::table
                .load(&mut conn)
                .await
                .unwrap();

        assert_eq!(rows.len(), 2);

        let today = start.format("%Y-%m-%d").to_string();
        for (date, user_id, app_id, closed_millis, open_millis, _extended) in &rows {
            assert_eq!(date, &today);
            assert_eq!(*user_id, 1000);
            assert_eq!(*open_millis, 0); // both intervals are closed
            match app_id.as_str() {
                // 10 min = 600 000 ms, 5 min = 300 000 ms
                "firefox" => assert_eq!(*closed_millis, 600_000),
                "code" => assert_eq!(*closed_millis, 300_000),
                other => panic!("unexpected app_id: {other}"),
            }
        }
    }

    #[tokio::test]
    async fn test_same_app_focus_does_not_buffer_unfocused() {
        let (_tmp, _pool, mut actor, _signal_rx) = setup().await;

        actor
            .handle_event(PlatformEvent::WindowFocused {
                app_id: app("firefox"),
                title: WindowTitle::new("test"),
                pid: Pid(1234),
                uid: Uid(1000),
                overlay_shown: false,
            })
            .await;

        actor
            .handle_event(PlatformEvent::WindowFocused {
                app_id: app("firefox"),
                title: WindowTitle::new("test"),
                pid: Pid(1234),
                uid: Uid(1000),
                overlay_shown: false,
            })
            .await;

        assert_eq!(actor.event_buffer.len(), 1);
        assert_eq!(actor.current_focus.get(&Uid(1000)), Some(&app("firefox")));
    }

    #[tokio::test]
    async fn test_recover_no_open_intervals() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let pool = StoreBuilder::new(tmp.path().to_path_buf())
            .build()
            .await
            .unwrap();
        let platform = Arc::new(MockPlatform);
        let (signal_tx, _) = mpsc::unbounded_channel();
        let active_blocks = Arc::new(RwLock::new(HashMap::new()));
        let (mut enforcer, _) = EnforcerActor::new(
            pool.clone(),
            platform,
            SystemClock,
            signal_tx,
            active_blocks,
        );

        let uid = Uid(1000);
        let ts_millis =
            chrono::NaiveDateTime::parse_from_str("2025-01-01 10:00:00", "%Y-%m-%d %H:%M:%S")
                .unwrap()
                .and_utc()
                .timestamp_millis();
        enforcer
            .repo
            .write_event(1, uid.0 as i32, ts_millis, Some("firefox"), None)
            .await
            .unwrap();

        enforcer.recover().await.unwrap();

        assert!(enforcer.current_focus.is_empty());
    }
}
