//! Generated D-Bus proxy trait for the wellbeing daemon Controller interface.
//!
//! The `#[proxy]` attribute generates `DaemonProxy<'static>` from the `Daemon`
//! trait. Re-exported from the parent module for use by `DaemonClient` and
//! `SignalCoalescer`.

use tracing::warn;
use wellbeing_core::*;
use zbus::proxy;
use zbus::zvariant::{OwnedValue, Value};

/// Wrapper around `Vec<BlockedAppEntry>` with a `TryFrom<OwnedValue>` impl
/// that strips the outer `Value::Value(...)` Variant layer added by
/// `Properties.Get`.
///
/// zbus 5.18's `Proxy::get_property` calls `T::try_from(OwnedValue)` directly,
/// but `Vec<T>::try_from` only handles `Value::Array` — not `Value::Value`
/// wrapping it. This wrapper unwraps the Variant before delegating.
#[derive(Debug, Clone)]
pub struct BlockedApps(pub Vec<BlockedAppEntry>);

impl std::convert::TryFrom<OwnedValue> for BlockedApps {
    type Error = zbus::Error;

    fn try_from(value: OwnedValue) -> std::result::Result<Self, Self::Error> {
        let inner = match Value::from(value) {
            Value::Value(v) => *v,
            other => other,
        };
        let entries = Vec::<BlockedAppEntry>::try_from(inner).map_err(|e| {
            let msg = e.to_string();
            warn!(error = %msg, "BlockedApps: failed to deserialize blocked_apps D-Bus property");
            zbus::Error::Failure(msg)
        })?;
        Ok(BlockedApps(entries))
    }
}

impl From<BlockedApps> for Vec<BlockedAppEntry> {
    fn from(w: BlockedApps) -> Self {
        w.0
    }
}

#[proxy(
    interface = "org.wellbeing.v1.Controller",
    default_service = "org.wellbeing.v1.Controller",
    default_path = "/org/wellbeing/Controller"
)]
pub trait Daemon {
    async fn list_policies(&self, filter_owner: u32) -> zbus::Result<Vec<PolicyData>>;
    async fn create_policy(&self, input: PolicyInput) -> zbus::Result<PolicyId>;
    async fn update_policy(&self, id: PolicyId, input: PolicyInput) -> zbus::Result<()>;
    async fn delete_policy(&self, id: PolicyId) -> zbus::Result<()>;

    /// Pre-aggregated per-app totals across a date range, sorted by
    /// total_millis DESC by the SQL query — no GUI-side sorting needed.
    async fn get_app_usage_summary(
        &self,
        start_date: &str,
        end_date: &str,
        user_id: u32,
    ) -> zbus::Result<Vec<AppUsageSummary>>;

    /// Pre-aggregated per-title totals across a date range, sorted by
    /// total_millis DESC by the SQL query — no GUI-side sorting needed.
    async fn get_title_usage_summary(
        &self,
        start_date: &str,
        end_date: &str,
        user_id: u32,
    ) -> zbus::Result<Vec<TitleUsageSummary>>;

    /// Pre-aggregated per-category totals across a date range, sorted by
    /// total_millis DESC from SQL — no GUI-side aggregation needed.
    async fn get_category_usage_summary(
        &self,
        start_date: &str,
        end_date: &str,
        user_id: u32,
    ) -> zbus::Result<Vec<CategoryUsageSummary>>;

    /// Per-date total usage across a range, pre-aggregated by SQL.
    /// One row per date — the bar chart uses this directly instead of
    /// flattening per-entry data and summing by date in memory.
    async fn get_daily_bar_totals(
        &self,
        start_date: &str,
        end_date: &str,
        user_id: u32,
    ) -> zbus::Result<Vec<DateTotal>>;

    async fn get_day_events(
        &self,
        uid: u32,
        start_millis: i64,
        end_millis: i64,
    ) -> zbus::Result<Vec<DayEventRow>>;

    async fn list_categories(&self) -> zbus::Result<Vec<Category>>;
    async fn get_app_categories(&self) -> zbus::Result<Vec<AppCategoryRow>>;
    async fn set_app_category(&self, app_class: &str, category_id: CategoryId) -> zbus::Result<()>;

    /// Read-only property — returns [`BlockedApps`] instead of raw `Vec` to work
    /// around zbus 5.18's Variant-unwrapping bug in `Proxy::get_property`.
    #[zbus(property)]
    fn blocked_apps(&self) -> zbus::Result<BlockedApps>;

    /// Signals (non-async — zbus generates receivers)
    #[zbus(signal, name = "BlockedAppsChanged")]
    fn on_blocked_apps_changed(&self) -> zbus::Result<BlockedAppsChangedSignal>;

    #[zbus(signal)]
    fn daily_usage_changed(&self) -> zbus::Result<Uid>;

    #[zbus(signal)]
    fn policy_mutated(&self) -> zbus::Result<u32>;
}
