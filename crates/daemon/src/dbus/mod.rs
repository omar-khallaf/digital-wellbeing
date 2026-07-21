use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use diesel::{ExpressionMethods, QueryDsl, delete, insert_into, update};
use diesel_async::RunQueryDsl;
use tokio::sync::{RwLock, mpsc::UnboundedSender};
use tracing::{info, warn};
use wellbeing_core::dbus_constants::DAEMON_OBJECT_PATH;
use wellbeing_core::{
    ActiveBlockEntry, AppCategoryRow, AppId, Category, CategoryId, Clock, DailySummary,
    DailyUsageEntry, PluginInstanceId, Policy, PolicyInput, Uid,
};
use zbus::fdo;
use zbus::interface;
use zbus::object_server::SignalEmitter;

use crate::platform::PlatformEvent;
use crate::platform::linux::{ManagerProxy, PluginRegistry};
use crate::policy::PolicyRow;
use crate::store::DbPool;
use crate::store::schema::{app_categories, categories, daily_usage, policies};

pub struct DaemonInterface {
    pool: DbPool,
    registry: Arc<RwLock<PluginRegistry>>,
    event_tx: UnboundedSender<PlatformEvent>,
    plugin_reg_cooldown: RwLock<HashMap<u32, Instant>>,
    clock: Box<dyn Clock>,
    active_blocks: Arc<RwLock<HashMap<Uid, HashMap<AppId, ActiveBlockEntry>>>>,
}

impl DaemonInterface {
    pub fn new(
        pool: DbPool,
        registry: Arc<RwLock<PluginRegistry>>,
        event_tx: UnboundedSender<PlatformEvent>,
        clock: Box<dyn Clock>,
        active_blocks: Arc<RwLock<HashMap<Uid, HashMap<AppId, ActiveBlockEntry>>>>,
    ) -> Self {
        Self {
            pool,
            registry,
            event_tx,
            plugin_reg_cooldown: RwLock::new(HashMap::new()),
            clock,
            active_blocks,
        }
    }

    async fn authenticate(
        conn: &zbus::Connection,
        header: zbus::message::Header<'_>,
    ) -> Result<u32, zbus::fdo::Error> {
        let sender = header.sender().ok_or_else(|| {
            tracing::error!("no sender in message header");
            fdo::Error::Failed("internal error".into())
        })?;

        let dbus_proxy = fdo::DBusProxy::new(conn).await.map_err(|e| {
            tracing::error!(error = %e, "failed to create DBusProxy");
            fdo::Error::Failed("internal error".into())
        })?;

        let creds = dbus_proxy
            .get_connection_credentials(sender.clone().into())
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "failed to get connection credentials");
                fdo::Error::Failed("internal error".into())
            })?;

        creds.unix_user_id().ok_or_else(|| {
            tracing::error!("no unix uid in caller credentials");
            fdo::Error::Failed("internal error".into())
        })
    }

    fn resolve_uid(caller: u32, target: u32) -> u32 {
        if caller == 0 { target } else { caller }
    }
}

#[interface(name = "org.wellbeing.v1.Controller")]
impl DaemonInterface {
    async fn list_policies(
        &self,
        filter_owner: u32,
        #[zbus(connection)] conn: &zbus::Connection,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> zbus::fdo::Result<Vec<Policy>> {
        let caller = Self::authenticate(conn, header).await?;
        let uid = Self::resolve_uid(caller, filter_owner);

        let mut db = self.pool.get().await.map_err(|e| {
            tracing::error!(error = %e, "db error");
            fdo::Error::Failed("internal error".into())
        })?;

        let rows: Vec<PolicyRow> = if caller == 0 {
            policies::table.load(&mut db).await
        } else {
            policies::table
                .filter(policies::owner_id.eq(uid as i32))
                .load(&mut db)
                .await
        }
        .map_err(|e| {
            tracing::error!(error = %e, "query failed");
            fdo::Error::Failed("internal error".into())
        })?;

        Ok(rows
            .into_iter()
            .map(PolicyRow::into_domain_policy)
            .collect())
    }

    async fn create_policy(
        &self,
        input: PolicyInput,
        #[zbus(connection)] conn: &zbus::Connection,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> fdo::Result<wellbeing_core::PolicyId> {
        let caller = Self::authenticate(conn, header).await?;
        if caller != 0 && input.owner_id != caller {
            return Err(fdo::Error::AccessDenied("access denied".into()));
        }

        let mut db = self.pool.get().await.map_err(|e| {
            tracing::error!(error = %e, "db error");
            fdo::Error::Failed("internal error".into())
        })?;

        let now = self.clock.now().format("%Y-%m-%d %H:%M:%S").to_string();

        let category_id: Option<i32> = if input.category_id > 0 {
            Some(input.category_id as i32)
        } else {
            None
        };
        let app_id: Option<String> = if input.app_id.is_empty() {
            None
        } else {
            Some(input.app_id.clone())
        };
        let time_limit: Option<i32> = if input.time_limit_seconds > 0 {
            Some(input.time_limit_seconds as i32)
        } else {
            None
        };
        let notify_repeat: Option<i32> = if input.notification_repeat_interval_seconds > 0 {
            Some(input.notification_repeat_interval_seconds as i32)
        } else {
            None
        };

        insert_into(policies::table)
            .values((
                policies::name.eq(&input.name),
                policies::kind.eq(input.kind as i32),
                policies::category_id.eq(category_id),
                policies::app_id.eq(app_id),
                policies::created_by.eq(caller as i32),
                policies::owner_id.eq(input.owner_id as i32),
                policies::time_limit_seconds.eq(time_limit),
                policies::extra_seconds.eq(input.extra_seconds as i32),
                policies::notification_repeat_interval_seconds.eq(notify_repeat),
                policies::schedule_json.eq(&input.schedule_json),
                policies::active.eq(input.active),
                policies::created_at.eq(&now),
                policies::updated_at.eq(&now),
            ))
            .execute(&mut db)
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "insert failed");
                fdo::Error::Failed("internal error".into())
            })?;

        use diesel::dsl::sql;
        let last_id: i32 = diesel::select(sql::<diesel::sql_types::Integer>("last_insert_rowid()"))
            .get_result(&mut db)
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "get last id failed");
                fdo::Error::Failed("internal error".into())
            })?;

        if let Ok(emitter) = SignalEmitter::new(conn, DAEMON_OBJECT_PATH)
            && let Err(e) = Self::policy_mutated(emitter, input.owner_id).await
        {
            tracing::error!(error = %e, "failed to emit policy_mutated signal");
        }
        Ok(wellbeing_core::PolicyId(last_id as i64))
    }

    async fn update_policy(
        &self,
        id: wellbeing_core::PolicyId,
        input: PolicyInput,
        #[zbus(connection)] conn: &zbus::Connection,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> fdo::Result<()> {
        let caller = Self::authenticate(conn, header).await?;
        let policy_id = id.0 as i32;

        let mut db = self.pool.get().await.map_err(|e| {
            tracing::error!(error = %e, "db error");
            fdo::Error::Failed("internal error".into())
        })?;

        let now = self.clock.now().format("%Y-%m-%d %H:%M:%S").to_string();

        let category_id: Option<i32> = if input.category_id > 0 {
            Some(input.category_id as i32)
        } else {
            None
        };
        let app_id: Option<String> = if input.app_id.is_empty() {
            None
        } else {
            Some(input.app_id)
        };
        let time_limit: Option<i32> = if input.time_limit_seconds > 0 {
            Some(input.time_limit_seconds as i32)
        } else {
            None
        };
        let notify_repeat: Option<i32> = if input.notification_repeat_interval_seconds > 0 {
            Some(input.notification_repeat_interval_seconds as i32)
        } else {
            None
        };

        let changes = (
            policies::name.eq(&input.name),
            policies::kind.eq(input.kind as i32),
            policies::category_id.eq(category_id),
            policies::app_id.eq(app_id),
            policies::time_limit_seconds.eq(time_limit),
            policies::extra_seconds.eq(input.extra_seconds as i32),
            policies::notification_repeat_interval_seconds.eq(notify_repeat),
            policies::schedule_json.eq(&input.schedule_json),
            policies::active.eq(input.active),
            policies::updated_at.eq(&now),
        );

        let rows = if caller != 0 {
            update(
                policies::table
                    .filter(policies::id.eq(policy_id))
                    .filter(policies::owner_id.eq(caller as i32)),
            )
            .set(changes)
            .execute(&mut db)
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "update failed");
                fdo::Error::Failed("internal error".into())
            })?
        } else {
            update(policies::table.filter(policies::id.eq(policy_id)))
                .set(changes)
                .execute(&mut db)
                .await
                .map_err(|e| {
                    tracing::error!(error = %e, "update failed");
                    fdo::Error::Failed("internal error".into())
                })?
        };

        if rows == 0 {
            return Err(fdo::Error::Failed("policy not found".into()));
        }

        if let Ok(emitter) = SignalEmitter::new(conn, DAEMON_OBJECT_PATH)
            && let Err(e) = Self::policy_mutated(emitter, input.owner_id).await
        {
            tracing::error!(error = %e, "failed to emit policy_mutated signal");
        }
        Ok(())
    }

    async fn delete_policy(
        &self,
        id: wellbeing_core::PolicyId,
        #[zbus(connection)] conn: &zbus::Connection,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> fdo::Result<()> {
        let caller = Self::authenticate(conn, header).await?;
        let policy_id = id.0 as i32;

        let mut db = self.pool.get().await.map_err(|e| {
            tracing::error!(error = %e, "db error");
            fdo::Error::Failed("internal error".into())
        })?;

        let policy_owner: i32 = policies::table
            .filter(policies::id.eq(policy_id))
            .select(policies::owner_id)
            .first(&mut db)
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "query failed");
                fdo::Error::Failed("internal error".into())
            })?;

        if caller != 0 && policy_owner != caller as i32 {
            return Err(fdo::Error::AccessDenied("access denied".into()));
        }

        let rows = delete(policies::table.filter(policies::id.eq(policy_id)))
            .execute(&mut db)
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "delete failed");
                fdo::Error::Failed("internal error".into())
            })?;

        if rows == 0 {
            return Err(fdo::Error::Failed("policy not found".into()));
        }

        if let Ok(emitter) = SignalEmitter::new(conn, DAEMON_OBJECT_PATH)
            && let Err(e) = Self::policy_mutated(emitter, policy_owner as u32).await
        {
            tracing::error!(error = %e, "failed to emit policy_mutated signal");
        }
        Ok(())
    }

    async fn register_plugin(
        &self,
        #[zbus(connection)] conn: &zbus::Connection,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> fdo::Result<()> {
        // Extract sender unique bus name BEFORE authenticate consumes header.
        let sender_str = header
            .sender()
            .ok_or_else(|| {
                warn!("register_plugin: no sender in message header");
                fdo::Error::Failed("internal error".into())
            })?
            .as_ref()
            .to_owned();

        let caller_uid = Self::authenticate(conn, header).await?;
        let uid = Uid(caller_uid);
        let instance = PluginInstanceId::new(&sender_str);

        {
            let cooldown = self.plugin_reg_cooldown.read().await;
            if let Some(last) = cooldown.get(&caller_uid)
                && last.elapsed() < std::time::Duration::from_secs(10)
            {
                warn!(?uid, "rate limited: plugin registration too frequent");
                return Err(fdo::Error::Failed("rate limited".into()));
            }
        }

        info!(?uid, ?instance, "plugin registering");

        // Use the unique bus name (":1.xxx") as proxy destination.
        // Plugin no longer claims a well-known name — the daemon connects
        // via the unique name assigned by the bus daemon.
        let builder = ManagerProxy::builder(conn)
            .destination(sender_str)
            .map_err(|e| {
                warn!(?uid, "failed to set destination: {e}");
                fdo::Error::Failed("plugin proxy creation failed".into())
            })?;
        let proxy = builder.build().await.map_err(|e| {
            warn!(?uid, "failed to build plugin proxy: {e}");
            fdo::Error::Failed("plugin proxy creation failed".into())
        })?;

        let instance_clone = instance.clone();
        {
            let mut reg = self.registry.write().await;
            reg.register(instance, uid, proxy);
        }

        let reg = self.registry.read().await;
        if let Some(ev_rx) = reg.subscribe_signals(&instance_clone).await {
            let ev_tx = self.event_tx.clone();
            tokio::spawn(async move {
                use futures::StreamExt;
                let mut stream = tokio_stream::wrappers::ReceiverStream::new(ev_rx);
                while let Some(event) = stream.next().await {
                    if ev_tx.send(event).is_err() {
                        info!("plugin event channel closed");
                        break;
                    }
                }
            });
        }

        {
            let mut cooldown = self.plugin_reg_cooldown.write().await;
            cooldown.insert(caller_uid, Instant::now());
        }

        Ok(())
    }

    #[zbus(property)]
    async fn active_blocks(&self) -> fdo::Result<Vec<wellbeing_core::ActiveBlockEntry>> {
        let blocks = self.active_blocks.read().await;
        let mut result = Vec::new();
        for uid_blocks in blocks.values() {
            for entry in uid_blocks.values() {
                result.push(wellbeing_core::ActiveBlockEntry {
                    app_id: entry.app_id.clone(),
                    policy_id: entry.policy_id,
                    blocked_since: entry.blocked_since,
                    reason: entry.reason,
                    available_actions: entry.available_actions.clone(),
                });
            }
        }
        Ok(result)
    }

    async fn get_daily_usage(
        &self,
        date: String,
        user_id: u32,
        #[zbus(connection)] conn: &zbus::Connection,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> fdo::Result<Vec<DailyUsageEntry>> {
        let caller = Self::authenticate(conn, header).await?;
        let uid = Self::resolve_uid(caller, user_id);

        let mut db = self.pool.get().await.map_err(|e| {
            tracing::error!(error = %e, "db error");
            fdo::Error::Failed("internal error".into())
        })?;

        let rows: Vec<(String, i32, String, i32, bool)> = daily_usage::table
            .filter(daily_usage::date.eq(&date))
            .filter(daily_usage::user_id.eq(uid as i32))
            .select((
                daily_usage::date,
                daily_usage::user_id,
                daily_usage::app_id,
                daily_usage::total_seconds,
                daily_usage::extended,
            ))
            .load(&mut db)
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "query failed");
                fdo::Error::Failed("internal error".into())
            })?;

        Ok(rows
            .into_iter()
            .map(|(d, uid, app_id, secs, ext)| DailyUsageEntry {
                date: d,
                user_id: uid as u32,
                app_id,
                total_seconds: secs as i64,
                extended: ext,
            })
            .collect())
    }

    async fn get_usage_range(
        &self,
        start_date: String,
        end_date: String,
        user_id: u32,
        #[zbus(connection)] conn: &zbus::Connection,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> fdo::Result<Vec<DailySummary>> {
        let caller = Self::authenticate(conn, header).await?;
        let uid = Self::resolve_uid(caller, user_id);

        let mut db = self.pool.get().await.map_err(|e| {
            tracing::error!(error = %e, "db error");
            fdo::Error::Failed("internal error".into())
        })?;

        let rows: Vec<(String, i32, String, i32, bool)> = daily_usage::table
            .filter(daily_usage::date.ge(&start_date))
            .filter(daily_usage::date.le(&end_date))
            .filter(daily_usage::user_id.eq(uid as i32))
            .select((
                daily_usage::date,
                daily_usage::user_id,
                daily_usage::app_id,
                daily_usage::total_seconds,
                daily_usage::extended,
            ))
            .load(&mut db)
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "query failed");
                fdo::Error::Failed("internal error".into())
            })?;

        let mut grouped: HashMap<String, Vec<DailyUsageEntry>> = HashMap::new();
        for (d, uid, app_id, secs, ext) in rows {
            grouped.entry(d.clone()).or_default().push(DailyUsageEntry {
                date: d,
                user_id: uid as u32,
                app_id,
                total_seconds: secs as i64,
                extended: ext,
            });
        }

        let mut summaries: Vec<DailySummary> = grouped
            .into_iter()
            .map(|(date, entries)| {
                let user_id = entries.as_slice().first().map(|e| e.user_id).unwrap_or(uid);
                DailySummary {
                    date,
                    user_id,
                    entries,
                }
            })
            .collect();

        summaries.sort_by(|a, b| a.date.cmp(&b.date));
        Ok(summaries)
    }

    async fn list_categories(&self) -> fdo::Result<Vec<Category>> {
        let mut db = self.pool.get().await.map_err(|e| {
            tracing::error!(error = %e, "db error");
            fdo::Error::Failed("internal error".into())
        })?;

        let rows: Vec<(i32, String, Option<String>, Option<String>)> = categories::table
            .select((
                categories::id,
                categories::name,
                categories::color,
                categories::icon,
            ))
            .load(&mut db)
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "query failed");
                fdo::Error::Failed("internal error".into())
            })?;

        Ok(rows
            .into_iter()
            .map(|(id, name, color, icon)| Category {
                id: CategoryId(id as i64),
                name,
                color: color.unwrap_or_default(),
                icon: icon.unwrap_or_default(),
            })
            .collect())
    }

    async fn get_app_categories(
        &self,
        #[zbus(connection)] conn: &zbus::Connection,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> fdo::Result<Vec<AppCategoryRow>> {
        let caller = Self::authenticate(conn, header).await?;

        let mut db = self.pool.get().await.map_err(|e| {
            tracing::error!(error = %e, "db error");
            fdo::Error::Failed("internal error".into())
        })?;

        type AppCategoryRowRaw = (
            String,
            i32,
            Option<i32>,
            Option<String>,
            Option<String>,
            bool,
        );
        let rows: Vec<AppCategoryRowRaw> = app_categories::table
            .filter(app_categories::user_id.eq(caller as i32))
            .select((
                app_categories::app_id,
                app_categories::user_id,
                app_categories::category_id,
                app_categories::display_name,
                app_categories::icon_path,
                app_categories::ignore,
            ))
            .load(&mut db)
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "query failed");
                fdo::Error::Failed("internal error".into())
            })?;

        Ok(rows
            .into_iter()
            .map(
                |(app_id, user_id, cat_id, display_name, icon_path, ignore)| AppCategoryRow {
                    app_id,
                    user_id: user_id as u32,
                    category_id: cat_id.unwrap_or(0) as i64,
                    display_name: display_name.unwrap_or_default(),
                    icon_path: icon_path.unwrap_or_default(),
                    ignore,
                },
            )
            .collect())
    }

    async fn set_app_category(
        &self,
        app_id: String,
        category_id: wellbeing_core::CategoryId,
        #[zbus(connection)] conn: &zbus::Connection,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> fdo::Result<()> {
        let caller = Self::authenticate(conn, header).await?;
        let uid = caller as i32;
        tracing::info!(caller, "set_app_category");

        let mut db = self.pool.get().await.map_err(|e| {
            tracing::error!(error = %e, "db error");
            fdo::Error::Failed("internal error".into())
        })?;

        let now = self.clock.now().format("%Y-%m-%d %H:%M:%S").to_string();
        let cat_id: Option<i32> = if category_id.0 > 0 {
            Some(category_id.0 as i32)
        } else {
            None
        };

        let updated = update(
            app_categories::table
                .filter(app_categories::app_id.eq(&app_id))
                .filter(app_categories::user_id.eq(uid)),
        )
        .set((
            app_categories::category_id.eq(cat_id),
            app_categories::updated_at.eq(&now),
        ))
        .execute(&mut db)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "update failed");
            fdo::Error::Failed("internal error".into())
        })?;

        if updated == 0 {
            insert_into(app_categories::table)
                .values((
                    app_categories::app_id.eq(&app_id),
                    app_categories::user_id.eq(uid),
                    app_categories::category_id.eq(cat_id),
                    app_categories::display_name.eq(None::<String>),
                    app_categories::icon_path.eq(None::<String>),
                    app_categories::ignore.eq(false),
                    app_categories::updated_at.eq(&now),
                ))
                .execute(&mut db)
                .await
                .map_err(|e| {
                    tracing::error!(error = %e, "insert failed");
                    fdo::Error::Failed("internal error".into())
                })?;
        }

        if let Ok(emitter) = SignalEmitter::new(conn, DAEMON_OBJECT_PATH)
            && let Err(e) = Self::policy_mutated(emitter, caller).await
        {
            tracing::error!(error = %e, "failed to emit policy_mutated signal from set_app_category");
        }

        Ok(())
    }

    /// A policy was created, updated, or deleted.
    #[zbus(signal)]
    async fn policy_mutated(emitter: SignalEmitter<'_>, uid: u32) -> zbus::Result<()>;

    /// Daily usage data mutated for a user.
    #[zbus(signal)]
    async fn daily_usage_changed(emitter: SignalEmitter<'_>, uid: u32) -> zbus::Result<()>;

    /// Block state changed for an app (shown / hidden).
    #[zbus(signal)]
    async fn block_state_changed(
        emitter: SignalEmitter<'_>,
        data: (u32, String, bool, u32),
    ) -> zbus::Result<()>;
}
