//! Policy CRUD authorization helpers.
//!
//! Extracts the duplicate owner-verification pattern from update_policy
//! and delete_policy into a single `verify_policy_owner` helper.

use zbus::fdo;

use crate::policy::data::PolicyRepo;

/// Look up the owner of a policy by ID and verify that the caller
/// is authorized to modify/delete it.
///
/// Returns `Ok(owner_id)` on success — the caller is either root (uid 0)
/// or the policy's actual owner.
///
/// Returns `Err(fdo::Error::AccessDenied)` if a non-root caller tries to
/// act on another user's policy.
pub(crate) async fn verify_policy_owner(
    repo: &PolicyRepo,
    policy_id: i32,
    caller: u32,
) -> Result<i32, fdo::Error> {
    let owner_id = repo.get_owner(policy_id).await.map_err(|e| {
        tracing::error!(error = %e, "query failed");
        fdo::Error::Failed("internal error".into())
    })?;
    if caller != 0 && owner_id != caller as i32 {
        return Err(fdo::Error::AccessDenied("access denied".into()));
    }
    Ok(owner_id)
}
