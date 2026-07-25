//! D-Bus signal subscription and coalescing.
//!
//! Listens for daemon signals (`blocked_apps_changed`, `daily_usage_changed`,
//! `policy_mutated`) and forwards coalesced notifications to the GPUI thread.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use futures::StreamExt;
use tokio::sync::mpsc;
use tracing::warn;

use super::client::DaemonProxy;

/// Coalesces rapid-fire D-Bus signals into periodic cache invalidations.
#[derive(Debug)]
pub struct SignalCoalescer {
    blocked_dirty: AtomicBool,
    usage_dirty: AtomicBool,
    policy_dirty: AtomicBool,
}

/// Bitmask of dirty flags returned by `SignalCoalescer::drain()`.
#[derive(Debug, Clone, Copy, Default)]
pub struct CoalescedNotifications {
    pub blocked: bool,
    pub usage: bool,
    pub policy: bool,
}

impl CoalescedNotifications {
    pub fn any(&self) -> bool {
        self.blocked || self.usage || self.policy
    }
}

impl SignalCoalescer {
    pub fn new() -> Self {
        Self {
            blocked_dirty: AtomicBool::new(false),
            usage_dirty: AtomicBool::new(false),
            policy_dirty: AtomicBool::new(false),
        }
    }

    pub fn mark_blocked_dirty(&self) {
        self.blocked_dirty.store(true, Ordering::Release);
    }

    pub fn mark_daily_usage_dirty(&self) {
        self.usage_dirty.store(true, Ordering::Release);
    }

    pub fn mark_policy_dirty(&self) {
        self.policy_dirty.store(true, Ordering::Release);
    }

    pub fn drain(&self) -> CoalescedNotifications {
        CoalescedNotifications {
            blocked: self.blocked_dirty.swap(false, Ordering::AcqRel),
            usage: self.usage_dirty.swap(false, Ordering::AcqRel),
            policy: self.policy_dirty.swap(false, Ordering::AcqRel),
        }
    }
}

impl Default for SignalCoalescer {
    fn default() -> Self {
        Self::new()
    }
}

pub fn spawn_signal_listener(
    client: &super::client::DaemonClient,
    coalescer: Arc<SignalCoalescer>,
    signal_tx: mpsc::UnboundedSender<CoalescedNotifications>,
) {
    let conn = client.connection().clone();
    tokio::spawn(async move {
        let proxy = match DaemonProxy::new(&conn).await {
            Ok(p) => p,
            Err(e) => {
                warn!(%e, "failed signal proxy");
                return;
            }
        };

        let tx = signal_tx.clone();
        let coal = coalescer.clone();
        if let Ok(mut stream) = proxy.receive_on_blocked_apps_changed().await {
            tokio::spawn(async move {
                while let Some(msg) = stream.next().await {
                    if msg
                        .message()
                        .body()
                        .deserialize::<(u32, String, bool, u32)>()
                        .is_ok()
                    {
                        coal.mark_blocked_dirty();
                        let _ = tx.send(CoalescedNotifications {
                            blocked: true,
                            ..Default::default()
                        });
                    }
                }
            });
        }

        let tx = signal_tx.clone();
        let coal = coalescer.clone();
        if let Ok(mut stream) = proxy.receive_daily_usage_changed().await {
            tokio::spawn(async move {
                while let Some(msg) = stream.next().await {
                    if msg.message().body().deserialize::<u32>().is_ok() {
                        coal.mark_daily_usage_dirty();
                        let _ = tx.send(CoalescedNotifications {
                            usage: true,
                            ..Default::default()
                        });
                    }
                }
            });
        }

        let coal = coalescer.clone();
        if let Ok(mut stream) = proxy.receive_policy_mutated().await {
            tokio::spawn(async move {
                while let Some(msg) = stream.next().await {
                    if msg.message().body().deserialize::<u32>().is_ok() {
                        coal.mark_policy_dirty();
                        let _ = signal_tx.send(CoalescedNotifications {
                            policy: true,
                            ..Default::default()
                        });
                    }
                }
            });
        }
    });
}
