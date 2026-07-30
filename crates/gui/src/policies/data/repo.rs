//! PoliciesRepository — timeout-guarded D-Bus reads + writes with boundary validation.
//!
//! Read methods wrap zbus proxy calls in `tokio::time::timeout`.  Write methods
//! (create/update/delete policy) are unbounded — they are user-triggered and
//! expected to complete within a few hundred ms over local D-Bus.

use std::time::Duration;

use anyhow::Result;
use tokio::time::timeout;
use tracing::warn;
use wellbeing_core::*;

use crate::dbus::BusManager;
use crate::dbus::client::DaemonProxy;

const DBUS_TIMEOUT: Duration = Duration::from_secs(10);

/// Repository for policies D-Bus queries and mutations.
#[derive(Debug, Clone)]
pub struct PoliciesRepo {
    pub(crate) bus: BusManager,
}

impl PoliciesRepo {
    pub fn new(bus: BusManager) -> Self {
        Self { bus }
    }

    pub(crate) async fn proxy(&self) -> Result<DaemonProxy<'static>> {
        self.bus.create_proxy().await
    }

    pub async fn list_policies(&self, uid: u32) -> Result<Vec<PolicyData>> {
        let proxy = self.proxy().await?;
        timeout(DBUS_TIMEOUT, proxy.list_policies(uid))
            .await
            .map_err(|_| anyhow::anyhow!("timeout: list_policies"))?
            .map_err(Into::into)
    }

    pub async fn list_categories(&self) -> Result<Vec<Category>> {
        let proxy = self.proxy().await?;
        timeout(DBUS_TIMEOUT, proxy.list_categories())
            .await
            .map_err(|_| anyhow::anyhow!("timeout: list_categories"))?
            .map_err(Into::into)
    }

    pub async fn get_app_categories(&self) -> Result<Vec<AppCategoryRow>> {
        let proxy = self.proxy().await?;
        timeout(DBUS_TIMEOUT, proxy.get_app_categories())
            .await
            .map_err(|_| anyhow::anyhow!("timeout: get_app_categories"))?
            .map_err(Into::into)
    }

    /// Fetch all data needed to build a `PoliciesViewModel`.
    pub async fn fetch_all(&self, uid: u32) -> Result<PoliciesData> {
        let (policies, categories, app_cats) = tokio::join!(
            self.list_policies(uid),
            self.list_categories(),
            self.get_app_categories(),
        );

        Ok(PoliciesData {
            policies: policies.unwrap_or_else(|e| {
                warn!("policies: list failed: {e}");
                vec![]
            }),
            categories: categories.unwrap_or_else(|e| {
                warn!("policies: categories failed: {e}");
                vec![]
            }),
            app_list: app_cats.unwrap_or_else(|e| {
                warn!("policies: app_categories failed: {e}");
                vec![]
            }),
        })
    }

    pub async fn create_policy(&self, input: PolicyInput) -> Result<PolicyId> {
        let proxy = self.proxy().await?;
        proxy.create_policy(input).await.map_err(Into::into)
    }

    pub async fn update_policy(&self, id: PolicyId, input: PolicyInput) -> Result<()> {
        let proxy = self.proxy().await?;
        proxy.update_policy(id, input).await.map_err(Into::into)
    }

    pub async fn delete_policy(&self, id: PolicyId) -> Result<()> {
        let proxy = self.proxy().await?;
        proxy.delete_policy(id).await.map_err(Into::into)
    }

    pub async fn set_app_category(&self, app_class: &AppClass, category: Category) -> Result<()> {
        let proxy = self.proxy().await?;
        proxy
            .set_app_category(app_class.as_ref(), category)
            .await
            .map_err(Into::into)
    }
}

/// Raw D-Bus response bundle for the policies screen.
#[derive(Debug, Clone, PartialEq)]
pub struct PoliciesData {
    pub policies: Vec<PolicyData>,
    pub categories: Vec<Category>,
    pub app_list: Vec<AppCategoryRow>,
}
