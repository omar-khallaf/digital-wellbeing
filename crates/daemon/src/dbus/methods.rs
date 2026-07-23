//! D-Bus interface methods — each public method is ≤20 lines.

use wellbeing_core::{
    ActiveBlockEntry, AppCategoryRow, Category, DailySummary, DailyUsageEntry, PluginInstanceId,
    PolicyData, PolicyInput, Uid,
};
use zbus::fdo;
use zbus::interface;

use crate::platform::{PlatformEvent, linux::ManagerProxy};

use super::controller::DaemonInterface;
use super::core::{authenticate, resolve_uid};
use super::data;
use super::signals;

#[interface(name = "org.wellbeing.v1.Controller")]
impl DaemonInterface {
    async fn list_policies(
        &self,
        filter_owner: u32,
        #[zbus(connection)] conn: &zbus::Connection,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> zbus::fdo::Result<Vec<PolicyData>> {
        let caller = authenticate(conn, header).await?;
        let uid = resolve_uid(caller, filter_owner);
        data::list_policies(&self.pool, caller == 0, uid as i32)
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "db error");
                fdo::Error::Failed("internal error".into())
            })
    }

    async fn create_policy(
        &self,
        input: PolicyInput,
        #[zbus(connection)] conn: &zbus::Connection,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> fdo::Result<wellbeing_core::PolicyId> {
        let caller = authenticate(conn, header).await?;
        if caller != 0 && input.owner_id != caller {
            return Err(fdo::Error::AccessDenied("access denied".into()));
        }
        let now = self.clock.now().format("%Y-%m-%d %H:%M:%S").to_string();
        let id = data::create_policy(&self.pool, input, caller, &now)
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "insert failed");
                fdo::Error::Failed("internal error".into())
            })?;
        let _ = signals::policy_mutated(conn, caller).await;
        Ok(wellbeing_core::PolicyId(id))
    }

    async fn update_policy(
        &self,
        id: wellbeing_core::PolicyId,
        input: PolicyInput,
        #[zbus(connection)] conn: &zbus::Connection,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> fdo::Result<()> {
        let caller = authenticate(conn, header).await?;
        let policy_id = id.0 as i32;
        let owner_id = data::get_policy_owner(&self.pool, policy_id)
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "query failed");
                fdo::Error::Failed("internal error".into())
            })?;
        if caller != 0 && owner_id != caller as i32 {
            return Err(fdo::Error::AccessDenied("access denied".into()));
        }
        let now = self.clock.now().format("%Y-%m-%d %H:%M:%S").to_string();
        let updated = data::update_policy(&self.pool, id, input, &now)
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "update failed");
                fdo::Error::Failed("internal error".into())
            })?;
        if !updated {
            return Err(fdo::Error::Failed("policy not found".into()));
        }
        let _ = signals::policy_mutated(conn, owner_id as u32).await;
        Ok(())
    }

    async fn delete_policy(
        &self,
        id: wellbeing_core::PolicyId,
        #[zbus(connection)] conn: &zbus::Connection,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> fdo::Result<()> {
        let caller = authenticate(conn, header).await?;
        let policy_id = id.0 as i32;
        let owner_id = data::get_policy_owner(&self.pool, policy_id)
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "query failed");
                fdo::Error::Failed("internal error".into())
            })?;
        if caller != 0 && owner_id != caller as i32 {
            return Err(fdo::Error::AccessDenied("access denied".into()));
        }
        let deleted = data::delete_policy(&self.pool, policy_id)
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "delete failed");
                fdo::Error::Failed("internal error".into())
            })?;
        if !deleted {
            return Err(fdo::Error::Failed("policy not found".into()));
        }
        let _ = signals::policy_mutated(conn, owner_id as u32).await;
        Ok(())
    }

    async fn register_plugin(
        &self,
        #[zbus(connection)] conn: &zbus::Connection,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> fdo::Result<()> {
        let sender_str = header
            .sender()
            .ok_or_else(|| fdo::Error::Failed("no sender".into()))?
            .as_ref()
            .to_owned();
        let caller_uid = authenticate(conn, header).await?;
        let uid = Uid(caller_uid);
        let instance = PluginInstanceId::new(&sender_str);

        {
            let cooldown = self.plugin_reg_cooldown.read().await;
            if let Some(last) = cooldown.get(&caller_uid)
                && last.elapsed() < std::time::Duration::from_secs(10)
            {
                return Err(fdo::Error::Failed("rate limited".into()));
            }
        }

        let builder = ManagerProxy::builder(conn)
            .destination(sender_str)
            .map_err(|_| fdo::Error::Failed("plugin proxy creation failed".into()))?;
        let proxy = builder
            .build()
            .await
            .map_err(|_| fdo::Error::Failed("plugin proxy build failed".into()))?;

        let instance_clone = instance.clone();
        {
            let mut reg = self.registry.write().await;
            reg.register(instance, uid, proxy);
        }

        // Sync session state: compare CurrentFocus with the last DB event
        // and emit the minimal events needed to bring the in-memory and
        // persisted state up to date. This ensures we don't double-insert
        // WindowFocused when an interval is already open.
        sync_focus_on_register(&self.pool, &self.registry, &self.event_tx).await;

        // Subscribe to plugin signals and spawn the event forwarding loop
        // as a background task. IMPORTANT: the forwarding loop runs in a
        // spawned task instead of inline — the previous inline design caused
        // the D-Bus handler to block forever, timing out the plugin's
        // RegisterPlugin call with [org.freedesktop.DBus.Error.NoReply].
        let ev_rx = {
            let reg = self.registry.read().await;
            reg.subscribe_signals(&instance_clone, &self.tokio_handle)
                .await
        };

        if let Some(ev_rx) = ev_rx {
            let ev_tx = self.event_tx.clone();
            let handle = self.tokio_handle.clone();
            handle.spawn(async move {
                use futures::StreamExt;
                let mut stream = tokio_stream::wrappers::ReceiverStream::new(ev_rx);
                while let Some(event) = stream.next().await {
                    if ev_tx.send(event).is_err() {
                        break;
                    }
                }
            });
        }

        {
            let mut cooldown = self.plugin_reg_cooldown.write().await;
            cooldown.insert(caller_uid, std::time::Instant::now());
        }

        Ok(())
    }

    #[zbus(property)]
    async fn active_blocks(
        &self,
        #[zbus(connection)] conn: &zbus::Connection,
        #[zbus(header)] header: Option<zbus::message::Header<'_>>,
    ) -> fdo::Result<Vec<ActiveBlockEntry>> {
        let header = header.ok_or_else(|| fdo::Error::Failed("missing header".into()))?;
        let caller = authenticate(conn, header).await?;
        let blocks = self.active_blocks.read().await;
        let result: Vec<ActiveBlockEntry> = if caller == 0 {
            blocks.values().flat_map(|v| v.values().cloned()).collect()
        } else if let Some(uid_blocks) = blocks.get(&Uid(caller)) {
            uid_blocks.values().cloned().collect()
        } else {
            vec![]
        };
        Ok(result)
    }

    async fn get_daily_usage(
        &self,
        date: String,
        user_id: u32,
        #[zbus(connection)] conn: &zbus::Connection,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> fdo::Result<Vec<DailyUsageEntry>> {
        let caller = authenticate(conn, header).await?;
        let uid = resolve_uid(caller, user_id);
        data::get_daily_usage(&self.pool, &date, uid)
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "query failed");
                fdo::Error::Failed("internal error".into())
            })
    }

    async fn get_usage_range(
        &self,
        start_date: String,
        end_date: String,
        user_id: u32,
        #[zbus(connection)] conn: &zbus::Connection,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> fdo::Result<Vec<DailySummary>> {
        let caller = authenticate(conn, header).await?;
        let uid = resolve_uid(caller, user_id);
        data::get_usage_range(&self.pool, &start_date, &end_date, uid)
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "query failed");
                fdo::Error::Failed("internal error".into())
            })
    }

    async fn list_categories(&self) -> fdo::Result<Vec<Category>> {
        data::list_categories(&self.pool).await.map_err(|e| {
            tracing::error!(error = %e, "query failed");
            fdo::Error::Failed("internal error".into())
        })
    }

    async fn get_app_categories(
        &self,
        #[zbus(connection)] conn: &zbus::Connection,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> fdo::Result<Vec<AppCategoryRow>> {
        let caller = authenticate(conn, header).await?;
        data::get_app_categories(&self.pool, caller)
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "query failed");
                fdo::Error::Failed("internal error".into())
            })
    }

    async fn set_app_category(
        &self,
        app_id: String,
        category_id: wellbeing_core::CategoryId,
        #[zbus(connection)] conn: &zbus::Connection,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> fdo::Result<()> {
        let caller = authenticate(conn, header).await?;
        let now = self.clock.now().format("%Y-%m-%d %H:%M:%S").to_string();
        data::set_app_category(&self.pool, app_id, category_id, caller, &now)
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "update failed");
                fdo::Error::Failed("internal error".into())
            })?;
        let _ = signals::policy_mutated(conn, caller).await;
        Ok(())
    }
}

/// Sync session state after a plugin registers.
///
/// Compares `CurrentFocus` against the last event in the database and emits
/// only the minimal events needed to reconcile:
///
/// | DB last event        | CurrentFocus     | Action                                |
/// |----------------------|------------------|---------------------------------------|
/// | empty / close event  | WindowFocused    | send WindowFocused                    |
/// | empty / close event  | None             | nothing (desktop)                     |
/// | WindowFocused (same) | WindowFocused    | nothing (already open)                |
/// | WindowFocused (diff) | WindowFocused    | send Unfocused + WindowFocused        |
/// | WindowFocused        | None             | send Unfocused (close stale interval) |
pub(crate) async fn sync_focus_on_register(
    pool: &crate::store::DbPool,
    registry: &std::sync::Arc<tokio::sync::RwLock<crate::platform::linux::PluginRegistry>>,
    event_tx: &tokio::sync::mpsc::UnboundedSender<PlatformEvent>,
) {
    use crate::blocking::data::{CLOSE_EVENT_TYPES, EVENT_WINDOW_FOCUSED, EventRow};
    use crate::store::schema::events::dsl::*;
    use diesel::ExpressionMethods;
    use diesel::QueryDsl;
    use diesel::SelectableHelper;
    use diesel_async::RunQueryDsl;

    // 1. Query CurrentFocus from the plugin
    let current = {
        let reg = registry.read().await;
        reg.query_current_focus().await
    };

    // 2. Read the last event from the database
    let last_event: Option<EventRow> = match pool.get().await {
        Ok(mut conn) => events
            .order(id.desc())
            .select(EventRow::as_select())
            .first(&mut conn)
            .await
            .ok(),
        Err(e) => {
            tracing::error!(error = %e, "sync_focus_on_register: DB connection failed");
            return;
        }
    };

    // 3. Compare and act
    match (last_event, current) {
        // Empty DB or last event is a close → no open interval
        (None, Some(focus)) => {
            event_tx.send(focus).ok();
        }
        (Some(ref last), Some(focus)) if CLOSE_EVENT_TYPES.contains(&last.event_type) => {
            event_tx.send(focus).ok();
        }
        // Last event is WindowFocused and CurrentFocus has the same app → nothing
        (
            Some(EventRow {
                event_type: EVENT_WINDOW_FOCUSED,
                app_id: ref last_app,
                ..
            }),
            Some(PlatformEvent::WindowFocused {
                app_id: ref cur_app_id,
                ..
            }),
        ) if last_app.as_deref() == Some(cur_app_id.as_str()) => {
            // Same app — interval is already open, nothing to do.
        }
        // Last event is WindowFocused but CurrentFocus is different → close old, open new
        (
            Some(EventRow {
                event_type: EVENT_WINDOW_FOCUSED,
                ..
            }),
            Some(focus),
        ) => {
            event_tx.send(PlatformEvent::Unfocused).ok();
            event_tx.send(focus).ok();
        }
        // Last event is WindowFocused but plugin reports desktop → close stale interval
        (
            Some(EventRow {
                event_type: EVENT_WINDOW_FOCUSED,
                ..
            }),
            None,
        ) => {
            event_tx.send(PlatformEvent::Unfocused).ok();
        }
        // Last event was some other non-focus event (Idle, Resumed, etc.)
        (_, Some(focus)) => {
            event_tx.send(focus).ok();
        }
        // No app focused and no open interval — nothing to do.
        (_, None) => {}
    }
}
