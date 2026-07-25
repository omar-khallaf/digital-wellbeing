//! Background task spawn helpers for the daemon binary.
//!
//! Each function encapsulates a `tokio::spawn` call for an independent
//! background task.  Callers own the parameters and move them in.

use std::time::Duration;

use chrono::Utc;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::info;

use wellbeing_daemon::blocking::InternalEvent;
use wellbeing_daemon::store::DbPool;

/// Spawn the prune-loop background task that periodically removes
/// old usage records from the database.
pub(crate) fn spawn_prune_loop(pool: DbPool) {
    tokio::spawn(async move {
        wellbeing_daemon::reports::prune_loop(pool, Box::new(wellbeing_core::SystemClock)).await;
    });
}

/// Spawn a wall-clock aligned minute-ticker that sends
/// `InternalEvent::Flush(None)` to the enforcer every minute boundary.
///
/// Re-calculates the delay on every iteration so NTP steps / system
/// sleep do not cause drift.
pub(crate) fn spawn_minute_ticker(
    flush_tx: mpsc::Sender<InternalEvent>,
    shutdown_token: CancellationToken,
) {
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = shutdown_token.cancelled() => {
                    info!("minute-ticker: shutting down");
                    break;
                }
                _ = async {
                    let now = Utc::now();
                    let next_boundary = (now.timestamp() / 60 + 1) * 60;
                    let delay_secs = (next_boundary - now.timestamp()).max(1) as u64;
                    tokio::time::sleep(Duration::from_secs(delay_secs)).await;
                } => {
                    if flush_tx.send(InternalEvent::Flush(None)).await.is_err() {
                        info!("minute-ticker: enforcer actor dropped");
                        break;
                    }
                }
            }
        }
    });
}
