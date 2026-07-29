//! D-Bus [`#[interface]`](zbus::interface) method handlers for `DaemonInterface`.
//!
//! This module is split from the single `methods.rs` into a directory with
//! sub-module helpers for plugin registration (`plugin_handlers`),
//! policy CRUD authorization (`policy_handlers`), and query helpers
//! (`query_handlers`).

mod plugin_handlers;
mod policy_handlers;
mod query_handlers;

use wellbeing_core::{
    AppCategoryRow, AppClass, AppUsageSummary, BlockedAppEntry, Category, CategoryUsageSummary,
    DateTotal, DayEventRow, PluginInstanceId, PolicyData, PolicyInput, TitleUsageSummary, Uid,
};
use zbus::fdo;
use zbus::interface;

use crate::blocking::InternalEvent;
use crate::platform::linux::ManagerProxy;

use super::controller::DaemonInterface;
use super::core::{authenticate, resolve_uid};
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
        self.policy_repo
            .list(caller == 0, uid as i32)
            .await
            .map(|policies| policies.into_iter().map(PolicyData::from).collect())
            .map_err(|e| query_handlers::map_err(e, "list policies failed"))
    }

    async fn create_policy(
        &self,
        input: PolicyInput,
        #[zbus(connection)] conn: &zbus::Connection,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> fdo::Result<wellbeing_core::PolicyId> {
        let caller = authenticate(conn, header).await?;
        let caller_uid = Uid(caller);
        if caller != 0 && input.user_id != caller_uid {
            return Err(fdo::Error::AccessDenied("access denied".into()));
        }
        let id = self
            .policy_repo
            .create(input, caller)
            .await
            .map_err(|e| query_handlers::map_err(e, "insert failed"))?;
        let _ = signals::policy_mutated(conn, caller_uid).await;
        let _ = self
            .policy_tx
            .send(InternalEvent::PolicyMutated {
                owner_id: Uid(caller),
            })
            .await;
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
        let owner_id =
            policy_handlers::verify_policy_owner(&self.policy_repo, policy_id, caller).await?;
        let updated = self
            .policy_repo
            .update(id, input)
            .await
            .map_err(|e| query_handlers::map_err(e, "update failed"))?;
        if !updated {
            return Err(fdo::Error::Failed("policy not found".into()));
        }
        let owner_uid = Uid(owner_id as u32);
        let _ = signals::policy_mutated(conn, owner_uid).await;
        let _ = self
            .policy_tx
            .send(InternalEvent::PolicyMutated {
                owner_id: owner_uid,
            })
            .await;
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
        let owner_id =
            policy_handlers::verify_policy_owner(&self.policy_repo, policy_id, caller).await?;
        let deleted = self
            .policy_repo
            .delete(policy_id)
            .await
            .map_err(|e| query_handlers::map_err(e, "delete failed"))?;
        if !deleted {
            return Err(fdo::Error::Failed("policy not found".into()));
        }
        let owner_uid = Uid(owner_id as u32);
        let _ = signals::policy_mutated(conn, owner_uid).await;
        let _ = self
            .policy_tx
            .send(InternalEvent::PolicyMutated {
                owner_id: owner_uid,
            })
            .await;
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

        // IMPORTANT: the forwarding loop runs in a spawned task instead of
        // inline — the previous inline design caused the D-Bus handler to
        // block forever, timing out the plugin's RegisterPlugin call with
        // [org.freedesktop.DBus.Error.NoReply].
        let ev_rx = {
            let reg = self.registry.read().await;
            reg.subscribe_signals(&instance_clone, &self.tokio_handle)
                .await
        };

        if let Some(ev_rx) = ev_rx {
            let ev_tx = self.event_tx.clone();
            let handle = self.tokio_handle.clone();
            plugin_handlers::spawn_event_forwarder(handle, ev_rx, ev_tx);
        }

        {
            let mut cooldown = self.plugin_reg_cooldown.write().await;
            cooldown.insert(caller_uid, std::time::Instant::now());
        }

        Ok(())
    }

    #[zbus(property)]
    async fn blocked_apps(
        &self,
        #[zbus(connection)] conn: &zbus::Connection,
        #[zbus(header)] header: Option<zbus::message::Header<'_>>,
    ) -> fdo::Result<Vec<BlockedAppEntry>> {
        use tracing::debug;
        let header = header.ok_or_else(|| fdo::Error::Failed("missing header".into()))?;
        let caller = authenticate(conn, header).await?;
        let blocks = self.blocked_apps.read().await;
        let result: Vec<BlockedAppEntry> = if caller == 0 {
            blocks.values().flat_map(|v| v.values()).cloned().collect()
        } else if let Some(uid_blocks) = blocks.get(&Uid(caller)) {
            uid_blocks.values().cloned().collect()
        } else {
            vec![]
        };
        #[cfg(debug_assertions)]
        if !result.is_empty() {
            debug!(
                "blocked_apps property returning {} entries, first: app_class={}, policy_id={}, reason={:?}, blocked_since={}",
                result.len(),
                result[0].app_class.as_ref(),
                result[0].policy_id.0,
                result[0].reason,
                result[0].blocked_since,
            );
        }
        Ok(result)
    }

    async fn get_app_usage_summary(
        &self,
        start_date: String,
        end_date: String,
        user_id: u32,
        #[zbus(connection)] conn: &zbus::Connection,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> fdo::Result<Vec<AppUsageSummary>> {
        let caller = authenticate(conn, header).await?;
        let uid = resolve_uid(caller, user_id);
        self.reports_repo
            .get_app_usage_summary(&start_date, &end_date, uid)
            .await
            .map_err(|e| query_handlers::map_err(e, "query failed"))
    }

    async fn get_title_usage_summary(
        &self,
        start_date: String,
        end_date: String,
        user_id: u32,
        #[zbus(connection)] conn: &zbus::Connection,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> fdo::Result<Vec<TitleUsageSummary>> {
        let caller = authenticate(conn, header).await?;
        let uid = resolve_uid(caller, user_id);
        self.reports_repo
            .get_title_usage_summary(&start_date, &end_date, uid)
            .await
            .map_err(|e| query_handlers::map_err(e, "query failed"))
    }

    async fn get_category_usage_summary(
        &self,
        start_date: String,
        end_date: String,
        user_id: u32,
        #[zbus(connection)] conn: &zbus::Connection,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> fdo::Result<Vec<CategoryUsageSummary>> {
        let caller = authenticate(conn, header).await?;
        let uid = resolve_uid(caller, user_id);
        self.reports_repo
            .get_category_usage_summary(&start_date, &end_date, uid)
            .await
            .map_err(|e| query_handlers::map_err(e, "query failed"))
    }

    async fn get_daily_bar_totals(
        &self,
        start_date: String,
        end_date: String,
        user_id: u32,
        #[zbus(connection)] conn: &zbus::Connection,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> fdo::Result<Vec<DateTotal>> {
        let caller = authenticate(conn, header).await?;
        let uid = resolve_uid(caller, user_id);
        self.reports_repo
            .get_daily_bar_totals(&start_date, &end_date, uid)
            .await
            .map_err(|e| query_handlers::map_err(e, "query failed"))
    }

    async fn get_day_events(
        &self,
        uid: u32,
        start_millis: i64,
        end_millis: i64,
        #[zbus(connection)] conn: &zbus::Connection,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> fdo::Result<Vec<DayEventRow>> {
        let caller = authenticate(conn, header).await?;
        let resolved_uid = Uid(resolve_uid(caller, uid));
        self.reports_repo
            .get_day_events(resolved_uid, start_millis, end_millis)
            .await
            .map_err(|e| query_handlers::map_err(e, "query failed"))
    }

    async fn list_categories(&self) -> fdo::Result<Vec<Category>> {
        self.categorization_repo
            .list_categories()
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "query failed");
                fdo::Error::Failed("internal error".into())
            })
    }

    async fn get_app_categories(
        &self,
        #[zbus(connection)] conn: &zbus::Connection,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> fdo::Result<Vec<AppCategoryRow>> {
        let caller = Uid(authenticate(conn, header).await?);
        self.categorization_repo
            .list_app_categories(caller)
            .await
            .map_err(|e| query_handlers::map_err(e, "query failed"))
    }

    async fn set_app_category(
        &self,
        app_class: String,
        category: wellbeing_core::Category,
        #[zbus(connection)] conn: &zbus::Connection,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> fdo::Result<()> {
        let caller = Uid(authenticate(conn, header).await?);
        // Validate app_class at the boundary — reject empty.
        let app_class = AppClass::new(&app_class)
            .map_err(|_| fdo::Error::InvalidArgs("invalid app_class (empty)".into()))?;
        let now = self.clock.now().format("%Y-%m-%d %H:%M:%S").to_string();
        self.categorization_repo
            .set_app_category(&app_class, category, caller, &now)
            .await
            .map_err(|e| query_handlers::map_err(e, "update failed"))?;
        let _ = signals::policy_mutated(conn, caller).await;
        Ok(())
    }
}
