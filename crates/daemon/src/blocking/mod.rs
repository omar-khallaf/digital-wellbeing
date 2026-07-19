//! Blocking enforcement module.
//!
//! [`EnforcerActor`] is the core enforcement engine. It receives
//! [`PlatformEvent`]s, evaluates policies **gate-first** (before any event
//! is persisted), and manages overlays / limit-timers / notify-timers.
//!
//! ## Gate-First Discipline
//!
//! On `WindowFocused`, the actor resolves categories & policies, calls
//! [`evaluate`], and acts on the verdict **before** writing any event:
//!
//! - **Block** → show overlay, DON'T write `WindowFocused` for the app.
//!   The previous app's interval IS closed (`Unfocused` written).
//! - **Notify** → write events normally, send notification, start repeat timer.
//! - **Ok** → write events normally, start limit timer.
//!
//! ## Timer Architecture
//!
//! Limit and notification timers are `tokio::spawn` tasks that send messages
//! through an internal `mpsc` channel when they fire. The actor drains this
//! channel at the start of every [`handle_event`] call, ensuring serial
//! processing without `Arc<Mutex>` wrapping.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use chrono::Utc;
use diesel::ExpressionMethods;
use diesel::QueryDsl;
use diesel_async::AsyncConnection;
use diesel_async::RunQueryDsl;
use serde_json::json;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use crate::platform::{OverlayConfig, Platform, PlatformEvent};
use crate::policy::*;
use crate::signal::DaemonSignal;
use crate::store::{DbPool, schema};
use crate::tracking::FocusState;
use wellbeing_core::*;

const EVENT_WINDOW_FOCUSED: i32 = 0;
const EVENT_UNFOCUSED: i32 = 1;

enum InternalEvent {
    LimitReached(AppId),
    NotifyTick(AppId),
}

#[derive(Debug)]
pub struct NotifyTimerState {
    pub policy_id: PolicyId,
    pub repeat_interval: Duration,
    pub(crate) handle: tokio::task::JoinHandle<()>,
}

#[derive(Debug, Clone)]
pub enum BlockingState {
    Idle,
    OverlayShown {
        app_id: AppId,
        policy_id: PolicyId,
        blocked_since: SystemTime,
        uid: Uid,
    },
}

pub struct EnforcerActor<P: Platform> {
    pool: DbPool,
    platform: Arc<P>,
    focus_state: HashMap<Uid, FocusState>,
    active_window: Option<(Uid, AppId)>,
    limit_timers: HashMap<AppId, tokio::task::JoinHandle<()>>,
    notify_timers: HashMap<AppId, NotifyTimerState>,
    blocking_state: BlockingState,
    internal_tx: mpsc::Sender<InternalEvent>,
    internal_rx: mpsc::Receiver<InternalEvent>,
    signal_tx: mpsc::UnboundedSender<DaemonSignal>,
    clock: Box<dyn Clock>,
}

impl<P: Platform> EnforcerActor<P> {
    pub fn new(
        pool: DbPool,
        platform: Arc<P>,
        clock: Box<dyn Clock>,
        signal_tx: mpsc::UnboundedSender<DaemonSignal>,
    ) -> Self {
        let (internal_tx, internal_rx) = mpsc::channel::<InternalEvent>(32);
        Self {
            pool,
            platform,
            focus_state: HashMap::new(),
            active_window: None,
            limit_timers: HashMap::new(),
            notify_timers: HashMap::new(),
            blocking_state: BlockingState::Idle,
            internal_tx,
            internal_rx,
            signal_tx,
            clock,
        }
    }

    pub async fn handle_event(&mut self, event: PlatformEvent) {
        self.drain_internal().await;

        match event {
            PlatformEvent::WindowFocused {
                app_id,
                title: _,
                pid: _,
                uid,
                overlay_shown: _,
            } => {
                self.handle_window_focused(app_id, uid).await;
            }
            PlatformEvent::Unfocused => {
                self.handle_unfocused().await;
            }
            PlatformEvent::Idle => {
                self.handle_idle().await;
            }
            PlatformEvent::Resumed => {
                self.handle_resumed().await;
            }
            PlatformEvent::Slept => {
                self.handle_idle().await;
            }
            PlatformEvent::ShutDown => {
                self.handle_unfocused().await;
            }
            PlatformEvent::Locked => {
                self.handle_idle().await;
            }
            PlatformEvent::LoggedOut => {
                self.handle_unfocused().await;
            }
            PlatformEvent::UserAction {
                app_id,
                action,
                policy_id,
            } => {
                let act = match action {
                    0 => OverlayAction::Extra,
                    1 => OverlayAction::Close,
                    _ => {
                        warn!(action, "Unknown overlay action value");
                        return;
                    }
                };
                self.handle_user_action(app_id, act, policy_id).await;
            }
        }
    }

    pub fn blocking_state(&self) -> &BlockingState {
        &self.blocking_state
    }

    async fn drain_internal(&mut self) {
        use tokio::sync::mpsc::error::TryRecvError;
        loop {
            match self.internal_rx.try_recv() {
                Ok(InternalEvent::LimitReached(app_id)) => {
                    self.on_limit_reached(app_id).await;
                }
                Ok(InternalEvent::NotifyTick(app_id)) => {
                    self.on_notify_tick(app_id).await;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => break,
            }
        }
    }

    async fn handle_window_focused(&mut self, app_id: AppId, uid: Uid) {
        let prev_app = self.active_window.as_ref().map(|(_, a)| a.clone());
        if let Some(ref prev_app_id) = prev_app
            && *prev_app_id != app_id
        {
            self.cancel_limit_timer(prev_app_id);
            self.cancel_notify_timer(prev_app_id);
        }

        let (verdict, policies, usage) = self.resolve_and_evaluate(&app_id, uid).await;

        match verdict {
            PolicyVerdict::Block {
                policy_id, reason, ..
            } => {
                self.enforce_block(&app_id, uid, &policies, policy_id, reason, usage)
                    .await;
            }
            PolicyVerdict::Notify {
                policy_id,
                repeat_interval,
            } => {
                self.handle_notify_verdict(
                    &app_id,
                    uid,
                    policy_id,
                    repeat_interval,
                    usage,
                    &policies,
                    &self.clock.now(),
                )
                .await;
            }
            PolicyVerdict::Ok => {
                self.handle_ok_verdict(&app_id, uid, usage, &policies, &self.clock.now())
                    .await;
            }
        }
    }

    async fn enforce_block(
        &mut self,
        app_id: &AppId,
        uid: Uid,
        policies: &[PolicyConfig],
        policy_id: PolicyId,
        reason: BlockReason,
        usage: (i64, bool),
    ) {
        if let Some(prev) = self.focus_state.remove(&uid) {
            self.close_interval(&uid, &prev).await;
        }

        self.cancel_limit_timer(app_id);

        let actions = self
            .determine_block_actions(policy_id, policies, usage)
            .await;
        self.show_block_overlay(app_id, uid, policy_id, reason, actions)
            .await;

        debug!(%app_id, "Block enforced — overlay shown");
    }

    #[allow(clippy::too_many_arguments)]
    async fn handle_notify_verdict(
        &mut self,
        app_id: &AppId,
        uid: Uid,
        policy_id: PolicyId,
        repeat_interval: Option<i64>,
        usage: (i64, bool),
        policies: &[PolicyConfig],
        now: &chrono::DateTime<Utc>,
    ) {
        self.open_new_interval(app_id, uid, now).await;

        self.active_window = Some((uid, app_id.clone()));

        let title = "Usage limit reached";
        let body = format!("{} has exceeded its usage limit.", app_id);
        if let Err(e) = self.platform.notify(title, &body).await {
            warn!(%app_id, error = %e, "Failed to send initial notification");
        }

        if let Some(repeat) = repeat_interval
            && repeat > 0
        {
            let interval = Duration::from_secs(repeat as u64);
            let delay = self.calculate_notify_delay(repeat, usage.0, interval);
            let state = NotifyTimerState {
                policy_id,
                repeat_interval: interval,
                handle: self.spawn_notify_handle(app_id.clone(), delay),
            };
            self.start_notify_timer(app_id.clone(), state);
        }

        self.start_limit_timer_if_needed(app_id, usage, policies);

        info!(%app_id, "Notify verdict handled");
    }

    async fn handle_ok_verdict(
        &mut self,
        app_id: &AppId,
        uid: Uid,
        usage: (i64, bool),
        policies: &[PolicyConfig],
        now: &chrono::DateTime<Utc>,
    ) {
        self.open_new_interval(app_id, uid, now).await;

        self.active_window = Some((uid, app_id.clone()));

        self.start_limit_timer_if_needed(app_id, usage, policies);

        debug!(%app_id, "Ok verdict handled — focus granted");
    }

    async fn handle_unfocused(&mut self) {
        let uids: Vec<Uid> = self.focus_state.keys().copied().collect();
        for uid in &uids {
            if let Some(prev) = self.focus_state.remove(uid) {
                self.close_interval(uid, &prev).await;
            }
        }

        self.cancel_all_timers();
        self.active_window = None;

        debug!("Unfocused — all intervals closed");
    }

    async fn handle_idle(&mut self) {
        let now = self.clock.now();
        for fs in self.focus_state.values_mut() {
            fs.pause(now);
        }
        debug!("Idle — intervals paused");
    }

    async fn handle_resumed(&mut self) {
        let now = self.clock.now();
        for fs in self.focus_state.values_mut() {
            fs.resume(now);
        }
        debug!("Resumed — intervals resumed");
    }

    pub async fn handle_user_action(
        &mut self,
        app_id: AppId,
        action: OverlayAction,
        policy_id: PolicyId,
    ) {
        match action {
            OverlayAction::Extra => {
                self.grant_extension(&app_id, policy_id).await;
            }
            OverlayAction::Close => {
                self.hide_overlay_and_reset(&app_id).await;
            }
        }
    }

    pub async fn grant_extension(&mut self, app_id: &AppId, policy_id: PolicyId) {
        let now = self.clock.now();
        let now_str = now.format("%Y-%m-%d %H:%M:%S").to_string();
        let today = now.format("%Y-%m-%d").to_string();
        let Some(&(uid, _)) = self.active_window.as_ref() else {
            warn!("grant_extension: no active window, ignoring");
            return;
        };

        let payload = json!({"t": &now_str, "a": app_id.as_ref()}).to_string();
        if let Err(e) = self
            .write_event(EVENT_WINDOW_FOCUSED, &payload, uid.0 as i32)
            .await
        {
            error!(%app_id, error = %e, "Failed to write synthetic WindowFocused");
            return;
        }

        if let Err(e) = self
            .update_daily_usage_extended(app_id, &today, &now_str, uid)
            .await
        {
            error!(%app_id, error = %e, "Failed to mark daily_usage extended");
            return;
        }

        self.focus_state
            .insert(uid, FocusState::new(app_id.clone(), now));

        if let Some(pc) = self.fetch_policy(policy_id).await {
            let extended_limit = match pc {
                PolicyConfig::TimeLimit {
                    time_limit_seconds,
                    extra_seconds,
                    ..
                } => time_limit_seconds + extra_seconds,
                PolicyConfig::Block { .. } | PolicyConfig::Notify { .. } => {
                    warn!(%app_id, "fetch_policy returned non-TimeLimit for grant_extension");
                    return;
                }
            };
            let total_seconds = self.get_usage(app_id, uid).await.unwrap_or((0, false)).0;
            let remaining = (extended_limit - total_seconds).max(0) as u64;
            if remaining > 0 {
                self.start_limit_timer(app_id.clone(), remaining);
            }
        }

        self.hide_overlay_and_reset(app_id).await;

        info!(%app_id, "Extension granted");
    }

    fn start_limit_timer(&mut self, app_id: AppId, remaining_secs: u64) {
        self.cancel_limit_timer(&app_id);
        if remaining_secs == 0 {
            return;
        }
        let tx = self.internal_tx.clone();
        let app_id_clone = app_id.clone();
        let handle = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(remaining_secs)).await;
            let _ = tx.send(InternalEvent::LimitReached(app_id_clone)).await;
        });
        self.limit_timers.insert(app_id, handle);
    }

    fn cancel_limit_timer(&mut self, app_id: &AppId) {
        if let Some(handle) = self.limit_timers.remove(app_id) {
            handle.abort();
        }
    }

    async fn on_limit_reached(&mut self, app_id: AppId) {
        let is_still_focused = self
            .active_window
            .as_ref()
            .map(|(_, a)| a == &app_id)
            .unwrap_or(false);
        if !is_still_focused {
            return;
        }

        let uid = self
            .active_window
            .as_ref()
            .map(|(u, _)| *u)
            .unwrap_or(Uid(0));
        let categories = self
            .resolve_categories(&app_id, uid)
            .await
            .unwrap_or_default();
        let policies = self
            .resolve_policies(&app_id, &categories, uid)
            .await
            .unwrap_or_default();
        let db_usage = self.get_usage(&app_id, uid).await.unwrap_or((0, false));
        let now = self.clock.now();
        let in_memory_extra = self
            .focus_state
            .get(&uid)
            .map(|fs| fs.active_duration(&now))
            .unwrap_or(0);
        let actual_total = db_usage.0 + in_memory_extra;
        let actual_usage = (actual_total, db_usage.1);
        let verdict = evaluate(&app_id, &policies, actual_usage.0, actual_usage.1);

        match verdict {
            PolicyVerdict::Block {
                policy_id, reason, ..
            } => {
                info!(%app_id, "Limit timer fired — enforcing block");
                self.enforce_block(&app_id, uid, &policies, policy_id, reason, actual_usage)
                    .await;
            }
            PolicyVerdict::Notify { .. } | PolicyVerdict::Ok => {
                let remaining = self.calculate_remaining(&policies, actual_usage);
                if remaining > 0 {
                    self.start_limit_timer(app_id.clone(), remaining);
                }
            }
        }
    }

    fn start_limit_timer_if_needed(
        &mut self,
        app_id: &AppId,
        usage: (i64, bool),
        policies: &[PolicyConfig],
    ) {
        let remaining = self.calculate_remaining(policies, usage);
        if remaining > 0 {
            self.start_limit_timer(app_id.clone(), remaining);
        }
    }

    fn calculate_remaining(&self, policies: &[PolicyConfig], usage: (i64, bool)) -> u64 {
        policies
            .iter()
            .filter_map(|p| match p {
                PolicyConfig::TimeLimit {
                    time_limit_seconds,
                    extra_seconds,
                    ..
                } => {
                    let limit = if usage.1 {
                        time_limit_seconds + extra_seconds
                    } else {
                        *time_limit_seconds
                    };
                    Some((limit - usage.0).max(0) as u64)
                }
                _ => None,
            })
            .min()
            .unwrap_or(0)
    }

    fn start_notify_timer(&mut self, app_id: AppId, state: NotifyTimerState) {
        self.cancel_notify_timer(&app_id);
        self.notify_timers.insert(app_id, state);
    }

    fn cancel_notify_timer(&mut self, app_id: &AppId) {
        if let Some(timer) = self.notify_timers.remove(app_id) {
            timer.handle.abort();
        }
    }

    async fn on_notify_tick(&mut self, app_id: AppId) {
        let is_still_focused = self
            .active_window
            .as_ref()
            .map(|(_, a)| a == &app_id)
            .unwrap_or(false);
        if !is_still_focused {
            return;
        }

        let body = format!("{} is still past its usage limit.", app_id);
        if let Err(e) = self.platform.notify("Limit reached", &body).await {
            warn!(%app_id, error = %e, "Failed to send repeat notification");
        }

        let interval = self.notify_timers.get(&app_id).map(|t| t.repeat_interval);
        if let Some(interval) = interval {
            let new_handle = self.spawn_notify_handle(app_id.clone(), interval);
            if let Some(timer) = self.notify_timers.get_mut(&app_id) {
                timer.handle = new_handle;
            }
        }
    }

    fn calculate_notify_delay(
        &self,
        repeat_seconds: i64,
        total_seconds: i64,
        repeat_interval: Duration,
    ) -> Duration {
        let excess = (total_seconds - repeat_seconds).max(0);
        let modulo = excess % repeat_seconds;
        let delay_secs = repeat_seconds - modulo;
        if delay_secs <= 0 {
            repeat_interval
        } else {
            Duration::from_secs(delay_secs as u64)
        }
    }

    fn spawn_notify_handle(&self, app_id: AppId, delay: Duration) -> tokio::task::JoinHandle<()> {
        let tx = self.internal_tx.clone();
        tokio::spawn(async move {
            tokio::time::sleep(delay).await;
            let _ = tx.send(InternalEvent::NotifyTick(app_id)).await;
        })
    }

    fn cancel_all_timers(&mut self) {
        for (_, handle) in self.limit_timers.drain() {
            handle.abort();
        }
        for (_, timer) in self.notify_timers.drain() {
            timer.handle.abort();
        }
    }

    async fn resolve_categories(
        &self,
        app_id: &AppId,
        uid: Uid,
    ) -> anyhow::Result<Vec<CategoryId>> {
        let mut conn = self.pool.get().await?;
        resolve_categories_for_app(&mut conn, app_id, uid).await
    }

    async fn resolve_policies(
        &self,
        app_id: &AppId,
        categories: &[CategoryId],
        uid: Uid,
    ) -> anyhow::Result<Vec<PolicyConfig>> {
        let mut conn = self.pool.get().await?;
        let policies = resolve_policies_for_app(&mut conn, app_id, categories, uid).await?;
        Ok(filter_policies_by_schedule(policies, self.clock.now()))
    }

    async fn get_usage(&self, app_id: &AppId, uid: Uid) -> anyhow::Result<(i64, bool)> {
        let mut conn = self.pool.get().await?;
        Ok(
            get_daily_usage_for_app(&mut conn, app_id, uid, &*self.clock)
                .await?
                .unwrap_or((0, false)),
        )
    }

    async fn fetch_policy(&self, policy_id: PolicyId) -> Option<PolicyConfig> {
        use crate::policy::PolicyRow;
        use diesel::OptionalExtension;
        let mut conn = self.pool.get().await.ok()?;
        let row: Option<PolicyRow> = schema::policies::table
            .filter(schema::policies::id.eq(policy_id.0 as i32))
            .first(&mut conn)
            .await
            .optional()
            .ok()?;
        row.map(|r| PolicyConfig::from(r.into_domain_policy()))
    }

    async fn write_event(
        &self,
        event_type: i32,
        payload: &str,
        user_id: i32,
    ) -> anyhow::Result<()> {
        let mut conn = self.pool.get().await?;
        diesel::insert_into(schema::events::table)
            .values((
                schema::events::event_type.eq(event_type),
                schema::events::payload.eq(payload),
                schema::events::user_id.eq(user_id),
            ))
            .execute(&mut conn)
            .await?;
        Ok(())
    }

    async fn close_interval(&self, uid: &Uid, prev: &FocusState) {
        let now = self.clock.now();
        let now_str = now.format("%Y-%m-%d %H:%M:%S").to_string();

        match async {
            let mut conn = self.pool.get().await?;
            conn.transaction(async |conn| {
                accumulate_daily_usage(conn, prev, &now, &now_str, *uid).await?;

                let payload = json!({"t": &now_str}).to_string();
                diesel::insert_into(schema::events::table)
                    .values((
                        schema::events::event_type.eq(EVENT_UNFOCUSED),
                        schema::events::payload.eq(&payload),
                        schema::events::user_id.eq(uid.0 as i32),
                    ))
                    .execute(conn)
                    .await?;
                Ok::<_, anyhow::Error>(())
            })
            .await?;
            Ok::<_, anyhow::Error>(())
        }
        .await
        {
            Ok(_) => {}
            Err(e) => error!(error = %e, "Failed to close interval"),
        }
    }

    async fn persist_focus_switch(
        &self,
        app_id: &AppId,
        uid: Uid,
        now: &chrono::DateTime<Utc>,
        prev: Option<&FocusState>,
    ) -> anyhow::Result<()> {
        let now_str = now.format("%Y-%m-%d %H:%M:%S").to_string();
        let mut conn = self.pool.get().await?;
        conn.transaction(async |conn| {
            if let Some(p) = prev {
                accumulate_daily_usage(conn, p, now, &now_str, uid).await?;
                let p = json!({"t": &now_str}).to_string();
                diesel::insert_into(schema::events::table)
                    .values((
                        schema::events::event_type.eq(EVENT_UNFOCUSED),
                        schema::events::payload.eq(&p),
                        schema::events::user_id.eq(uid.0 as i32),
                    ))
                    .execute(conn)
                    .await?;
            }
            let p = json!({"t": &now_str, "a": app_id.as_ref()}).to_string();
            diesel::insert_into(schema::events::table)
                .values((
                    schema::events::event_type.eq(EVENT_WINDOW_FOCUSED),
                    schema::events::payload.eq(&p),
                    schema::events::user_id.eq(uid.0 as i32),
                ))
                .execute(conn)
                .await?;
            Ok::<_, anyhow::Error>(())
        })
        .await?;
        Ok(())
    }

    async fn open_new_interval(&mut self, app_id: &AppId, uid: Uid, now: &chrono::DateTime<Utc>) {
        let prev = self.focus_state.get(&uid).cloned();
        match self
            .persist_focus_switch(app_id, uid, now, prev.as_ref())
            .await
        {
            Ok(()) => {
                self.focus_state
                    .insert(uid, FocusState::new(app_id.clone(), *now));
            }
            Err(e) => error!(%app_id, error = %e, "Failed to persist focus switch"),
        }
    }

    async fn determine_block_actions(
        &self,
        policy_id: PolicyId,
        policies: &[PolicyConfig],
        usage: (i64, bool),
    ) -> Vec<OverlayAction> {
        match policies.iter().find(|p| p.id() == policy_id) {
            Some(pc) => match pc {
                PolicyConfig::Block { .. } => vec![OverlayAction::Close],
                PolicyConfig::TimeLimit { .. } => match app_state(usage, pc) {
                    TrackedApp::TimeLimited(ref tl) if tl.can_extend() => {
                        vec![OverlayAction::Extra, OverlayAction::Close]
                    }
                    _ => vec![OverlayAction::Close],
                },
                PolicyConfig::Notify { .. } => vec![OverlayAction::Close],
            },
            None => {
                warn!(?policy_id, "Blocking policy not found in fetched set");
                vec![OverlayAction::Close]
            }
        }
    }

    async fn show_block_overlay(
        &mut self,
        app_id: &AppId,
        uid: Uid,
        policy_id: PolicyId,
        reason: BlockReason,
        available_actions: Vec<OverlayAction>,
    ) {
        self.blocking_state = BlockingState::OverlayShown {
            app_id: app_id.clone(),
            policy_id,
            blocked_since: SystemTime::from(self.clock.now()),
            uid,
        };
        let config = OverlayConfig {
            app_id: app_id.clone(),
            policy_id,
            reason,
            blocked_since: SystemTime::from(self.clock.now()),
            available_actions,
        };
        if let Err(e) = self.platform.show_overlay(config, uid).await {
            warn!(%app_id, error = %e, "Failed to show overlay");
        }
        let _ = self.signal_tx.send(DaemonSignal::BlockStateChanged {
            uid: uid.0,
            app_id: app_id.clone(),
            blocked: true,
            reason: reason as u32,
        });
    }

    async fn resolve_and_evaluate(
        &self,
        app_id: &AppId,
        uid: Uid,
    ) -> (PolicyVerdict, Vec<PolicyConfig>, (i64, bool)) {
        let categories = self
            .resolve_categories(app_id, uid)
            .await
            .unwrap_or_else(|e| {
                error!(%app_id, error = %e, "Failed to resolve categories");
                Vec::new()
            });
        let policies = self
            .resolve_policies(app_id, &categories, uid)
            .await
            .unwrap_or_else(|e| {
                warn!(%app_id, error = %e, "Failed to resolve policies");
                Vec::new()
            });
        let usage = self.get_usage(app_id, uid).await.unwrap_or_else(|e| {
            warn!(%app_id, error = %e, "Failed to get daily usage");
            (0, false)
        });
        let verdict = evaluate(app_id, &policies, usage.0, usage.1);
        (verdict, policies, usage)
    }

    async fn update_daily_usage_extended(
        &self,
        app_id: &AppId,
        today: &str,
        now_str: &str,
        uid: Uid,
    ) -> anyhow::Result<()> {
        let mut conn = self.pool.get().await?;

        let affected = diesel::update(
            schema::daily_usage::table
                .filter(schema::daily_usage::date.eq(today))
                .filter(schema::daily_usage::user_id.eq(uid.0 as i32))
                .filter(schema::daily_usage::app_id.eq(app_id.as_ref())),
        )
        .set((
            schema::daily_usage::extended.eq(true),
            schema::daily_usage::updated_at.eq(now_str),
        ))
        .execute(&mut conn)
        .await?;

        if affected == 0 {
            diesel::insert_into(schema::daily_usage::table)
                .values((
                    schema::daily_usage::date.eq(today),
                    schema::daily_usage::user_id.eq(uid.0 as i32),
                    schema::daily_usage::app_id.eq(app_id.as_ref()),
                    schema::daily_usage::total_seconds.eq(0),
                    schema::daily_usage::extended.eq(true),
                    schema::daily_usage::updated_at.eq(now_str),
                ))
                .execute(&mut conn)
                .await?;
        }

        Ok(())
    }

    async fn hide_overlay_and_reset(&mut self, app_id: &AppId) {
        let uid = match &self.blocking_state {
            BlockingState::OverlayShown { uid, .. } => *uid,
            _ => {
                warn!(%app_id, "hide_overlay_and_reset: no overlay shown");
                return;
            }
        };
        self.blocking_state = BlockingState::Idle;
        if let Err(e) = self.platform.hide_overlay(app_id, uid).await {
            warn!(%app_id, error = %e, "Failed to hide overlay");
        }

        let _ = self.signal_tx.send(DaemonSignal::BlockStateChanged {
            uid: uid.0,
            app_id: app_id.clone(),
            blocked: false,
            reason: 0,
        });
    }
}

async fn accumulate_daily_usage(
    conn: &mut impl AsyncConnection<Backend = diesel::sqlite::Sqlite>,
    prev: &FocusState,
    now: &chrono::DateTime<Utc>,
    now_str: &str,
    uid: Uid,
) -> anyhow::Result<()> {
    let duration = prev.active_duration(now);
    if duration > 0 {
        let date = prev.started_at().format("%Y-%m-%d").to_string();
        let app_id_str = prev.app_id().as_ref();
        diesel::insert_into(schema::daily_usage::table)
            .values((
                schema::daily_usage::date.eq(&date),
                schema::daily_usage::user_id.eq(uid.0 as i32),
                schema::daily_usage::app_id.eq(app_id_str),
                schema::daily_usage::total_seconds.eq(duration as i32),
                schema::daily_usage::extended.eq(false),
                schema::daily_usage::updated_at.eq(&now_str),
            ))
            .on_conflict((
                schema::daily_usage::date,
                schema::daily_usage::user_id,
                schema::daily_usage::app_id,
            ))
            .do_update()
            .set((
                schema::daily_usage::total_seconds
                    .eq(schema::daily_usage::total_seconds + duration as i32),
                schema::daily_usage::updated_at.eq(&now_str),
            ))
            .execute(conn)
            .await?;
    }
    Ok(())
}
