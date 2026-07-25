//! Periodic pruning of old events and daily-usage rows.

use std::time::Duration;

use chrono::Duration as ChronoDuration;
use diesel_async::RunQueryDsl;
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
    use diesel::sql_query;
    use diesel::sql_types::{BigInt, Integer, Text};

    let mut conn = pool.get().await?;
    let now = clock.now();
    let cutoff_millis = now.timestamp_millis() - 90 * 86400 * 1000;
    let cutoff_date = (now - ChronoDuration::days(90))
        .format("%Y-%m-%d")
        .to_string();

    loop {
        let count = sql_query(
            "DELETE FROM events WHERE id IN (SELECT id FROM events WHERE timestamp < $1 LIMIT $2)",
        )
        .bind::<BigInt, _>(&cutoff_millis)
        .bind::<Integer, _>(500)
        .execute(&mut conn)
        .await?;
        if count < 500 {
            break;
        }
    }

    loop {
        let count = sql_query(
            "DELETE FROM daily_usage WHERE rowid IN (SELECT rowid FROM daily_usage WHERE date < $1 LIMIT $2)"
        )
            .bind::<Text, _>(&cutoff_date)
            .bind::<Integer, _>(500)
            .execute(&mut conn)
            .await?;
        if count < 500 {
            break;
        }
    }

    loop {
        let count = sql_query(
            "DELETE FROM daily_usage_by_title WHERE rowid IN (SELECT rowid FROM daily_usage_by_title WHERE date < $1 LIMIT $2)"
        )
            .bind::<Text, _>(&cutoff_date)
            .bind::<Integer, _>(500)
            .execute(&mut conn)
            .await?;
        if count < 500 {
            break;
        }
    }

    Ok(())
}
