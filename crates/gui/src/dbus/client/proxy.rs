//! Generated D-Bus proxy trait for the wellbeing daemon Controller interface.
//!
//! The `#[proxy]` attribute generates `DaemonProxy<'static>` from the `Daemon`
//! trait. Re-exported from the parent module for use by `DaemonClient` and
//! `SignalCoalescer`.

use wellbeing_core::*;
use zbus::proxy;

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

    async fn get_daily_usage(
        &self,
        date: &str,
        user_id: u32,
    ) -> zbus::Result<Vec<DailyUsageByAppEntry>>;
    async fn get_usage_range(
        &self,
        start_date: &str,
        end_date: &str,
        user_id: u32,
    ) -> zbus::Result<Vec<DailySummary>>;

    async fn get_daily_usage_by_title(
        &self,
        date: &str,
        user_id: u32,
    ) -> zbus::Result<Vec<DailyUsageByTitleEntry>>;
    async fn get_usage_range_by_title(
        &self,
        start_date: &str,
        end_date: &str,
        user_id: u32,
    ) -> zbus::Result<Vec<DailyUsageByTitleSummary>>;

    async fn get_usage_range_by_category(
        &self,
        start_date: &str,
        end_date: &str,
        user_id: u32,
    ) -> zbus::Result<Vec<DailyUsageByCategoryEntry>>;

    async fn get_day_events(
        &self,
        uid: u32,
        start_millis: i64,
        end_millis: i64,
    ) -> zbus::Result<Vec<DayEventRow>>;

    async fn list_categories(&self) -> zbus::Result<Vec<Category>>;
    async fn get_app_categories(&self) -> zbus::Result<Vec<AppCategoryRow>>;
    async fn set_app_category(&self, app_class: &str, category_id: CategoryId) -> zbus::Result<()>;

    /// Signals (non-async — zbus generates receivers)
    #[zbus(signal, name = "BlockedAppsChanged")]
    fn on_blocked_apps_changed(&self) -> zbus::Result<(Uid, AppClass, bool, BlockReason)>;

    #[zbus(signal)]
    fn daily_usage_changed(&self) -> zbus::Result<Uid>;

    #[zbus(signal)]
    fn policy_mutated(&self) -> zbus::Result<u32>;
}

/// Property accessor for `BlockedApps`.
///
/// `BlockedAppEntry` now derives `Value`, so `Vec<BlockedAppEntry>` satisfies
/// `TryFrom<OwnedValue>` required by the property getter.
impl DaemonProxy<'_> {
    pub async fn blocked_apps(&self) -> zbus::Result<Vec<BlockedAppEntry>> {
        self.inner().get_property("BlockedApps").await
    }
}
