//! `DaemonClient` query and mutation methods — policy CRUD, usage reports,
//! category management, and blocked-apps lookup.

use anyhow::Result;
use wellbeing_core::*;

use super::DaemonClient;

impl DaemonClient {
    pub async fn list_policies(&self, filter_owner: u32) -> Result<Vec<PolicyData>> {
        let key = format!("policies:{}", filter_owner);
        if let Some(cached) = self.policy_cache.get(&key) {
            return Ok(cached);
        }
        let policies = self.proxy.list_policies(filter_owner).await?;
        self.policy_cache.set(key, policies.clone());
        Ok(policies)
    }

    pub async fn create_policy(&self, input: PolicyInput) -> Result<PolicyId> {
        let id = self.proxy.create_policy(input).await?;
        self.policy_cache.clear();
        Ok(id)
    }

    pub async fn update_policy(&self, id: PolicyId, input: PolicyInput) -> Result<()> {
        self.proxy.update_policy(id, input).await?;
        self.policy_cache.clear();
        Ok(())
    }

    pub async fn delete_policy(&self, id: PolicyId) -> Result<()> {
        self.proxy.delete_policy(id).await?;
        self.policy_cache.clear();
        Ok(())
    }

    pub async fn get_usage_range(
        &self,
        start_date: &str,
        end_date: &str,
        user_id: u32,
    ) -> Result<Vec<DailySummary>> {
        let key = format!("range:{}:{}:{}", start_date, end_date, user_id);
        if let Some(cached) = self.range_cache.get(&key) {
            return Ok(cached);
        }
        let summaries = self
            .proxy
            .get_usage_range(start_date, end_date, user_id)
            .await?;
        self.range_cache.set(key, summaries.clone());
        Ok(summaries)
    }

    pub async fn get_daily_usage_by_title(
        &self,
        date: &str,
        user_id: u32,
    ) -> Result<Vec<DailyUsageByTitleEntry>> {
        let key = format!("title:{}:{}", date, user_id);
        if let Some(cached) = self.daily_title_cache.get(&key) {
            return Ok(cached);
        }
        let entries = self.proxy.get_daily_usage_by_title(date, user_id).await?;
        self.daily_title_cache.set(key, entries.clone());
        Ok(entries)
    }

    pub async fn get_usage_range_by_title(
        &self,
        start_date: &str,
        end_date: &str,
        user_id: u32,
    ) -> Result<Vec<DailyUsageByTitleSummary>> {
        let key = format!("range_by_title:{}:{}:{}", start_date, end_date, user_id);
        if let Some(cached) = self.range_by_title_cache.get(&key) {
            return Ok(cached);
        }
        let entries = self
            .proxy
            .get_usage_range_by_title(start_date, end_date, user_id)
            .await?;
        self.range_by_title_cache.set(key, entries.clone());
        Ok(entries)
    }

    pub async fn get_day_events(
        &self,
        uid: u32,
        start_millis: i64,
        end_millis: i64,
    ) -> Result<Vec<DayEventRow>> {
        let key = format!("day_events:{}:{}:{}", uid, start_millis, end_millis);
        if let Some(cached) = self.day_events_cache.get(&key) {
            return Ok(cached);
        }
        let events = self
            .proxy
            .get_day_events(uid, start_millis, end_millis)
            .await?;
        self.day_events_cache.set(key, events.clone());
        Ok(events)
    }

    pub async fn list_categories(&self) -> Result<Vec<Category>> {
        let key = "categories".into();
        if let Some(cached) = self.category_cache.get(&key) {
            return Ok(cached);
        }
        let cats = self.proxy.list_categories().await?;
        self.category_cache.set(key, cats.clone());
        Ok(cats)
    }

    pub async fn get_app_categories(&self) -> Result<Vec<AppCategoryRow>> {
        let key = "app_categories".into();
        if let Some(cached) = self.app_category_cache.get(&key) {
            return Ok(cached);
        }
        let rows = self.proxy.get_app_categories().await?;
        self.app_category_cache.set(key, rows.clone());
        Ok(rows)
    }

    pub async fn get_blocked_apps(&self) -> Result<Vec<BlockedAppEntry>> {
        self.proxy.blocked_apps().await.map_err(Into::into)
    }
}
