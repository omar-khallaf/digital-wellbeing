//! Periodic pruning of old events and daily-usage rows.

use std::time::Duration;

use chrono::Duration as ChronoDuration;
use wellbeing_core::Clock;

use crate::store::DbPool;

/// Periodic prune loop — runs every hour, deletes old events and daily_usage.
pub async fn prune_loop(pool: DbPool, clock: Box<dyn Clock>) {
    let mut interval = tokio::time::interval(Duration::from_secs(3600));
    interval.tick().await;

    loop {
        interval.tick().await;
        if let Err(e) = prune_cycle(&pool, &*clock).await {
            tracing::error!(error = %e, "prune cycle failed");
        }
    }
}

async fn prune_cycle(pool: &DbPool, clock: &dyn Clock) -> anyhow::Result<()> {
    let conn = pool.client().await?;
    let now = clock.now();
    let cutoff_millis = now.timestamp_millis() - 90 * 86400 * 1000;
    let cutoff_date = (now - ChronoDuration::days(90))
        .format("%Y-%m-%d")
        .to_string();

    // Delete old events in batches
    loop {
        let sql =
            "DELETE FROM events WHERE id IN (SELECT id FROM events WHERE timestamp < ?1 LIMIT ?2)";
        let result = conn.execute(sql, (cutoff_millis, 500)).await?;
        if result < 500 {
            break;
        }
    }

    // Delete old daily_usage_by_app rows in batches
    loop {
        let sql = "DELETE FROM daily_usage_by_app WHERE rowid IN (SELECT rowid FROM daily_usage_by_app WHERE date < ?1 LIMIT ?2)";
        let result = conn.execute(sql, (cutoff_date.as_str(), 500)).await?;
        if result < 500 {
            break;
        }
    }

    // Delete old daily_usage_by_title rows in batches
    loop {
        let sql = "DELETE FROM daily_usage_by_title WHERE rowid IN (SELECT rowid FROM daily_usage_by_title WHERE date < ?1 LIMIT ?2)";
        let result = conn.execute(sql, (cutoff_date.as_str(), 500)).await?;
        if result < 500 {
            break;
        }
    }

    Ok(())
}
