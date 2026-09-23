//! Generated D-Bus proxy trait for the wellbeing daemon Controller interface.
//!
//! The `#[proxy]` attribute generates `DaemonProxy<'static>` from the `Daemon`
//! trait. Re-exported from the parent module for use by `DaemonClient` and
//! `SignalCoalescer`.
//!
//! Uid-free surface — caller identity is derived from SO_PEERCRED
//! on the daemon side, so no method takes a uid/user_id parameter. Root-only
//! `*ForUser` variants take an explicit target uid.

use wellbeing_core::*;
use zbus::proxy;

#[proxy(
    interface = "org.wellbeing.v1.Controller",
    default_service = "org.wellbeing.v1.Controller",
    default_path = "/org/wellbeing/Controller"
)]
pub trait Daemon {
    async fn list_policies(&self) -> zbus::Result<Vec<PolicyData>>;
    async fn list_policies_for_user(&self, uid: u32) -> zbus::Result<Vec<PolicyData>>;
    async fn create_policy(&self, input: PolicyInput) -> zbus::Result<PolicyId>;
    async fn create_policy_for_user(&self, input: PolicyInput, uid: u32) -> zbus::Result<PolicyId>;
    async fn update_policy(&self, id: PolicyId, input: PolicyInput) -> zbus::Result<()>;
    async fn update_policy_for_user(
        &self,
        id: PolicyId,
        input: PolicyInput,
        uid: u32,
    ) -> zbus::Result<()>;
    async fn delete_policy(&self, id: PolicyId) -> zbus::Result<()>;
    async fn delete_policy_for_user(&self, id: PolicyId, uid: u32) -> zbus::Result<()>;

    /// Pre-aggregated per-app totals across a date range, sorted by
    /// total_millis DESC by the SQL query — no GUI-side sorting needed.
    async fn get_app_usage_summary(
        &self,
        start_date: &str,
        end_date: &str,
    ) -> zbus::Result<Vec<AppUsageSummary>>;
    async fn get_app_usage_summary_for_user(
        &self,
        start_date: &str,
        end_date: &str,
        uid: u32,
    ) -> zbus::Result<Vec<AppUsageSummary>>;

    /// Pre-aggregated per-title totals across a date range, sorted by
    /// total_millis DESC by the SQL query — no GUI-side sorting needed.
    async fn get_title_usage_summary(
        &self,
        start_date: &str,
        end_date: &str,
    ) -> zbus::Result<Vec<TitleUsageSummary>>;
    async fn get_title_usage_summary_for_user(
        &self,
        start_date: &str,
        end_date: &str,
        uid: u32,
    ) -> zbus::Result<Vec<TitleUsageSummary>>;

    /// Pre-aggregated per-category totals across a date range, sorted by
    /// total_millis DESC from SQL — no GUI-side aggregation needed.
    async fn get_category_usage_summary(
        &self,
        start_date: &str,
        end_date: &str,
    ) -> zbus::Result<Vec<CategoryUsageSummary>>;
    async fn get_category_usage_summary_for_user(
        &self,
        start_date: &str,
        end_date: &str,
        uid: u32,
    ) -> zbus::Result<Vec<CategoryUsageSummary>>;

    /// Per-date total usage across a range, pre-aggregated by SQL.
    /// One row per date — the bar chart uses this directly instead of
    /// flattening per-entry data and summing by date in memory.
    async fn get_daily_bar_totals(
        &self,
        start_date: &str,
        end_date: &str,
    ) -> zbus::Result<Vec<DateTotal>>;
    async fn get_daily_bar_totals_for_user(
        &self,
        start_date: &str,
        end_date: &str,
        uid: u32,
    ) -> zbus::Result<Vec<DateTotal>>;

    async fn get_day_events(
        &self,
        start_millis: i64,
        end_millis: i64,
    ) -> zbus::Result<Vec<DayEventRow>>;
    async fn get_day_events_for_user(
        &self,
        uid: u32,
        start_millis: i64,
        end_millis: i64,
    ) -> zbus::Result<Vec<DayEventRow>>;

    async fn get_blocked_apps(&self) -> zbus::Result<Vec<BlockedAppEntry>>;
    async fn get_blocked_apps_for_user(&self, uid: u32) -> zbus::Result<Vec<BlockedAppEntry>>;

    async fn list_categories(&self) -> zbus::Result<Vec<Category>>;
    async fn get_app_categories(&self) -> zbus::Result<Vec<AppCategoryRow>>;
    async fn set_app_category(&self, app_class: &str, category: Category) -> zbus::Result<()>;

    /// Signals (non-async — zbus generates receivers)
    #[zbus(signal)]
    fn app_blocked(&self) -> zbus::Result<AppBlockedSignal>;

    #[zbus(signal)]
    fn policy_changed(&self) -> zbus::Result<PolicyChangedSignal>;

    #[zbus(signal)]
    fn domain_blocked(&self) -> zbus::Result<DomainBlockedSignal>;

    #[zbus(signal)]
    fn usage_updated(&self) -> zbus::Result<UsageUpdatedSignal>;
}
