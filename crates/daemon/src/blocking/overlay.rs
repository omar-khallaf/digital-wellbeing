//! Overlay state management for the blocking enforcement feature.
//!
//! [`OverlayManager`] owns the blocking state machine, the shared active-
//! blocks map (for the compositor plugin), and the D-Bus signal sender.
//! It exposes pure operations — the actor calls these in response to
//! policy verdicts and user actions.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::SystemTime;

use tokio::sync::mpsc;
use tracing::warn;
use wellbeing_core::*;

use crate::policy::{PolicyConfig, TrackedApp, app_state};
use crate::signal::DaemonSignal;

use super::domain::BlockingState;

/// Manages block overlay state, the shared active-blocks map, and signals.
pub(crate) struct OverlayManager {
    active_blocks: Arc<tokio::sync::RwLock<HashMap<Uid, HashMap<AppId, ActiveBlockEntry>>>>,
    signal_tx: mpsc::UnboundedSender<DaemonSignal>,
}

impl OverlayManager {
    pub fn new(
        active_blocks: Arc<tokio::sync::RwLock<HashMap<Uid, HashMap<AppId, ActiveBlockEntry>>>>,
        signal_tx: mpsc::UnboundedSender<DaemonSignal>,
    ) -> Self {
        Self {
            active_blocks,
            signal_tx,
        }
    }

    /// Current blocking state, derived from the active blocks map.
    pub async fn blocking_state(&self) -> BlockingState {
        let blocks = self.active_blocks.read().await;
        for (&uid, app_blocks) in blocks.iter() {
            if let Some((app_id, entry)) = app_blocks.iter().next() {
                return BlockingState::OverlayShown {
                    app_id: app_id.clone(),
                    policy_id: PolicyId(entry.policy_id as i64),
                    blocked_since: std::time::UNIX_EPOCH
                        + std::time::Duration::from_millis(entry.blocked_since),
                    uid,
                };
            }
        }
        BlockingState::Idle
    }

    /// Look up the policy id for a currently-blocked app+uid combination.
    pub async fn lookup_policy_id(&self, uid: Uid, app_id: &AppId) -> PolicyId {
        self.active_blocks
            .read()
            .await
            .get(&uid)
            .and_then(|blocks| blocks.get(app_id))
            .map(|entry| PolicyId(entry.policy_id as i64))
            .unwrap_or(PolicyId(0))
    }

    /// Determine which overlay actions are available for a block verdict.
    pub fn determine_actions(
        &self,
        policy_id: PolicyId,
        policies: &[PolicyConfig],
        usage: (i64, bool),
    ) -> Vec<OverlayAction> {
        match policies.iter().find(|p| p.id() == policy_id) {
            Some(pc) => match pc {
                PolicyConfig::Block { .. } => vec![OverlayAction::Close],
                PolicyConfig::TimeLimit { .. } => match app_state(usage, pc) {
                    TrackedApp::TimeLimited(ref tl) if tl.can_extend() => {
                        vec![OverlayAction::Extra, OverlayAction::Close]
                    }
                    _ => vec![OverlayAction::Close],
                },
                PolicyConfig::Notify { .. } => vec![OverlayAction::Close],
            },
            None => {
                warn!(?policy_id, "Blocking policy not found in fetched set");
                vec![OverlayAction::Close]
            }
        }
    }

    // ── block / unblock ────────────────────────────────────────────

    /// Show the block overlay for an app: persist to active_blocks and signal.
    pub async fn show_block(
        &mut self,
        app_id: &AppId,
        uid: Uid,
        policy_id: PolicyId,
        reason: BlockReason,
        available_actions: Vec<OverlayAction>,
        now: SystemTime,
    ) {
        let entry = ActiveBlockEntry {
            app_id: app_id.as_ref().to_string(),
            policy_id: policy_id.0 as u64,
            reason: reason as u32,
            blocked_since: now
                .duration_since(SystemTime::UNIX_EPOCH)
                .expect("blocked_since after epoch")
                .as_millis() as u64,
            available_actions: available_actions.iter().map(|a| *a as u32).collect(),
        };

        self.active_blocks
            .write()
            .await
            .entry(uid)
            .or_default()
            .insert(app_id.clone(), entry);

        let _ = self.signal_tx.send(DaemonSignal::BlockStateChanged {
            uid: uid.0,
            app_id: app_id.clone(),
            blocked: true,
            reason: reason as u32,
        });
    }

    /// Remove the block overlay for an app.
    pub async fn unblock(&mut self, app_id: &AppId, uid: Uid) {
        self.active_blocks
            .write()
            .await
            .entry(uid)
            .or_default()
            .remove(app_id);

        let _ = self.signal_tx.send(DaemonSignal::BlockStateChanged {
            uid: uid.0,
            app_id: app_id.clone(),
            blocked: false,
            reason: 0,
        });
    }
}
