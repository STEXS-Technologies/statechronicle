//! Unique asset profile.
//!
//! Singleton resources with ownership rules (protocol §20.2). A unique asset
//! carries an `owner` and a `status` in its projected payload and moves through
//! an explicit transition table. Hard delete is never permitted: `burn` is the
//! only terminal destructive transition and it requires owner authorization.

use std::collections::BTreeMap;

use statechronicle_domain::intent::Operation;
use statechronicle_domain::state::StateProjection;
use statechronicle_domain::state_type::StateType;
use statechronicle_domain::status::Status;

use crate::error::ProfileError;
use crate::keys;
use crate::registry::{ProfileRules, input_amount, input_str, require_from, state_str};

/// Typed wire status names for a unique asset.
pub mod status {
    use statechronicle_domain::status::Status;

    status_set! {
        /// The asset is held by its owner and may transition normally.
        active => "active";
        /// The asset is locked and cannot be transferred, listed, or burned.
        locked => "locked";
        /// The asset is listed for sale.
        listed => "listed";
        /// The asset is held in escrow pending settlement.
        escrowed => "escrowed";
        /// The asset was redeemed (for example a voucher or ticket consumed).
        redeemed => "redeemed";
        /// The asset was burned (terminal).
        burned => "burned";
        /// The asset is frozen in a pending trade (only `trade.unlock`,
        /// `trade.settle`, and `asset.restrict` escape).
        trade_held => "trade_held";
        /// The asset is restricted by policy.
        restricted => "restricted";
        /// The asset is under quarantine.
        quarantined => "quarantined";
        /// The asset is in an unsupported state.
        unsupported => "unsupported";
        /// The asset is tombstoned (soft-deleted, terminal).
        tombstoned => "tombstoned";
    }
}

/// Typed operation constants accepted by the unique asset profile.
pub mod op {
    use statechronicle_domain::intent::Operation;

    op_set! {
        /// Mints a new unique asset.
        asset_mint => "asset.mint";
        /// Transfers ownership of an `active` asset.
        asset_transfer => "asset.transfer";
        /// Burns an `active` asset (terminal).
        asset_burn => "asset.burn";
        /// Locks an `active` asset.
        asset_lock => "asset.lock";
        /// Unlocks a `locked` asset.
        asset_unlock => "asset.unlock";
        /// Redeems a `listed` asset (terminal).
        asset_redeem => "asset.redeem";
        /// Lists an `active` asset for sale.
        asset_list => "asset.list";
        /// Delists a `listed` asset.
        asset_delist => "asset.delist";
        /// Places an `active` asset in escrow.
        asset_escrow => "asset.escrow";
        /// Releases an `escrowed` asset.
        asset_release => "asset.release";
        /// Attaches opaque content to an `active` asset.
        asset_attach_content => "asset.attach_content";
        /// Detaches opaque content from an `active` asset.
        asset_detach_content => "asset.detach_content";
        /// Updates metadata on an `active` asset.
        asset_update_metadata => "asset.update_metadata";
        /// Restricts an asset into an exceptional status.
        asset_restrict => "asset.restrict";
        /// Restores an exceptional status back to `active`.
        asset_restore => "asset.restore";
        /// Freezes an `active` asset into a pending trade.
        trade_lock => "trade.lock";
        /// Unlocks a `trade_held` asset to its owner.
        trade_unlock => "trade.unlock";
        /// Settles a `trade_held` asset to its new owner.
        trade_settle => "trade.settle";
    }

    op_slice! {
        /// Operations that MUST carry an authority binding (protocol §11.2,
        /// ADR-006 §36 Q5 / deferral item 4).
        authority_required => [ asset_transfer, asset_burn, trade_settle ];
    }
}

/// Rule set for [`StateType::UniqueAsset`] (protocol §20.2).
///
/// # Transition table
///
/// | operation | from | to |
/// |---|---|---|
/// | `asset.mint` | *(no prior state)* | `active` |
/// | `asset.transfer` | `active` | `active` |
/// | `asset.burn` | `active` | `burned` |
/// | `asset.lock` | `active` | `locked` |
/// | `asset.unlock` | `locked` | `active` |
/// | `asset.list` | `active` | `listed` |
/// | `asset.delist` | `listed` | `active` |
/// | `asset.escrow` | `active` | `escrowed` |
/// | `asset.release` | `escrowed` | `active` |
/// | `asset.redeem` | `listed` | `redeemed` |
/// | `asset.attach_content` | `active` | `active` |
/// | `asset.detach_content` | `active` | `active` |
/// | `asset.update_metadata` | `active` | `active` |
/// | `asset.restrict` | `active`/`locked`/`listed`/`escrowed`/`trade_held` | `restricted` |
/// | `asset.restore` | `restricted`/`quarantined`/`unsupported` | `active` |
/// | `trade.lock` | `active` | `trade_held` |
/// | `trade.unlock` | `trade_held` | `active` |
/// | `trade.settle` | `trade_held` | `active` |
///
/// `redeemed`, `burned`, and `tombstoned` are terminal: no operation leaves
/// them. Hard delete is never permitted for any state.
#[derive(Debug, Clone, Copy)]
pub struct UniqueAssetRules;

impl ProfileRules for UniqueAssetRules {
    fn state_type(&self) -> StateType {
        StateType::UniqueAsset
    }

    fn profile_id(&self) -> &'static str {
        "unique_asset"
    }

    fn allowed_operations(&self) -> &'static [Operation] {
        op::all()
    }

    fn requires_authority(&self, operation: &Operation) -> bool {
        op::authority_required().contains(operation)
    }

    fn check(
        &self,
        operation: &Operation,
        current: Option<&StateProjection>,
        inputs: &BTreeMap<String, serde_json::Value>,
    ) -> Result<(), ProfileError> {
        if !op::all().contains(operation) {
            return Err(ProfileError::UnknownOperation(String::from(
                operation.as_str(),
            )));
        }
        if operation == op::asset_mint() {
            check_mint(current, inputs)
        } else if operation == op::asset_transfer() {
            check_transfer(current, inputs)
        } else if operation == op::asset_burn() {
            check_burn(current, inputs)
        } else if operation == op::asset_lock() {
            single_from(current, "asset.lock", &[status::active().to_owned()])
        } else if operation == op::asset_unlock() {
            single_from(current, "asset.unlock", &[status::locked().to_owned()])
        } else if operation == op::asset_list() {
            single_from(current, "asset.list", &[status::active().to_owned()])
        } else if operation == op::asset_delist() {
            single_from(current, "asset.delist", &[status::listed().to_owned()])
        } else if operation == op::asset_escrow() {
            single_from(current, "asset.escrow", &[status::active().to_owned()])
        } else if operation == op::asset_release() {
            single_from(current, "asset.release", &[status::escrowed().to_owned()])
        } else if operation == op::asset_redeem() {
            single_from(current, "asset.redeem", &[status::listed().to_owned()])
        } else if operation == op::asset_attach_content() {
            single_from(
                current,
                "asset.attach_content",
                &[status::active().to_owned()],
            )
        } else if operation == op::asset_detach_content() {
            single_from(
                current,
                "asset.detach_content",
                &[status::active().to_owned()],
            )
        } else if operation == op::asset_update_metadata() {
            single_from(
                current,
                "asset.update_metadata",
                &[status::active().to_owned()],
            )
        } else if operation == op::asset_restrict() {
            single_from(
                current,
                "asset.restrict",
                &[
                    status::active().to_owned(),
                    status::locked().to_owned(),
                    status::listed().to_owned(),
                    status::escrowed().to_owned(),
                    status::trade_held().to_owned(),
                ],
            )
        } else if operation == op::asset_restore() {
            single_from(
                current,
                "asset.restore",
                &[
                    status::restricted().to_owned(),
                    status::quarantined().to_owned(),
                    status::unsupported().to_owned(),
                ],
            )
        } else if operation == op::trade_lock() {
            check_trade_lock(current, inputs)
        } else if operation == op::trade_unlock() {
            check_trade_unlock(current, inputs)
        } else if operation == op::trade_settle() {
            check_trade_settle(current, inputs)
        } else {
            Err(ProfileError::UnknownOperation(String::from(
                operation.as_str(),
            )))
        }
    }
}

/// Requires the operation's source state and returns the projection.
///
/// Delegates to [`require_from`], which reads the `status` field and enforces
/// the allowed source states.
///
/// # Errors
///
/// Returns [`ProfileError::InvalidTransition`] when the resource does not
/// exist or its status is not one of `allowed_from`.
fn single_from(
    current: Option<&StateProjection>,
    operation: &str,
    allowed_from: &[Status],
) -> Result<(), ProfileError> {
    require_from(current, operation, allowed_from)?;
    Ok(())
}

/// Validates `asset.mint`: the resource must not exist yet.
///
/// The initial owner is taken from the required `to_owner` input; the
/// resulting asset is `active`.
///
/// # Errors
///
/// Returns [`ProfileError::InvalidTransition`] when a resource already exists,
/// or [`ProfileError::InvalidInput`] when the `to_owner` input is missing,
/// non-string, or empty.
fn check_mint(
    current: Option<&StateProjection>,
    inputs: &BTreeMap<String, serde_json::Value>,
) -> Result<(), ProfileError> {
    if let Some(current) = current {
        let from = state_str(current, "status")?;
        return Err(ProfileError::InvalidTransition {
            from: String::from(from),
            operation: String::from("asset.mint"),
        });
    }
    input_str(inputs, "to_owner")?;
    Ok(())
}

/// Validates `asset.transfer`: an `active` asset owned by `from_owner`.
///
/// Requires a `to_owner` input naming the new owner and a `from_owner` input
/// equal to the current owner.
///
/// # Errors
///
/// Returns [`ProfileError::InvalidTransition`] when the resource does not
/// exist or is not `active`, [`ProfileError::InvalidInput`] when `to_owner`
/// or `from_owner` is missing/malformed, and
/// [`ProfileError::OwnershipMismatch`] when `from_owner` is not the current
/// owner.
fn check_transfer(
    current: Option<&StateProjection>,
    inputs: &BTreeMap<String, serde_json::Value>,
) -> Result<(), ProfileError> {
    let current = require_from(current, "asset.transfer", &[status::active().to_owned()])?;
    let from_owner = input_str(inputs, "from_owner")?;
    input_str(inputs, "to_owner")?;
    let owner = state_str(current, "owner")?;
    if from_owner != owner {
        return Err(ProfileError::OwnershipMismatch {
            expected: String::from(owner),
            actual: String::from(from_owner),
        });
    }
    Ok(())
}

/// Validates `asset.burn`: an `active` asset burned by its owner.
///
/// # Errors
///
/// Returns [`ProfileError::InvalidTransition`] when the resource does not
/// exist or is not `active`, [`ProfileError::InvalidInput`] when `from_owner`
/// is missing/malformed, and [`ProfileError::OwnershipMismatch`] when
/// `from_owner` is not the current owner.
fn check_burn(
    current: Option<&StateProjection>,
    inputs: &BTreeMap<String, serde_json::Value>,
) -> Result<(), ProfileError> {
    let current = require_from(current, "asset.burn", &[status::active().to_owned()])?;
    let from_owner = input_str(inputs, "from_owner")?;
    let owner = state_str(current, "owner")?;
    if from_owner != owner {
        return Err(ProfileError::OwnershipMismatch {
            expected: String::from(owner),
            actual: String::from(from_owner),
        });
    }
    Ok(())
}

/// Validates `trade.lock`: an `active` asset frozen into a pending trade.
///
/// Requires a `from_owner` input equal to the current owner and a `trade_id`
/// input naming the pending trade. After-state (computed by the executor):
/// owner preserved, status `trade_held`, payload gains `trade_id`.
///
/// # Errors
///
/// Returns [`ProfileError::InvalidTransition`] when the resource does not
/// exist or is not `active`, [`ProfileError::InvalidInput`] when `from_owner`
/// or `trade_id` is missing/malformed, and
/// [`ProfileError::OwnershipMismatch`] when `from_owner` is not the current
/// owner.
fn check_trade_lock(
    current: Option<&StateProjection>,
    inputs: &BTreeMap<String, serde_json::Value>,
) -> Result<(), ProfileError> {
    let current = require_from(current, "trade.lock", &[status::active().to_owned()])?;
    let from_owner = input_str(inputs, "from_owner")?;
    input_str(inputs, "trade_id")?;
    let owner = state_str(current, "owner")?;
    if from_owner != owner {
        return Err(ProfileError::OwnershipMismatch {
            expected: String::from(owner),
            actual: String::from(from_owner),
        });
    }
    Ok(())
}

/// Validates `trade.unlock`: returns a `trade_held` asset to its owner.
///
/// Requires a `trade_id` input matching the stored `trade_id` on the asset.
/// After-state (computed by the executor): owner preserved, status `active`,
/// `trade_id` dropped.
///
/// # Errors
///
/// Returns [`ProfileError::InvalidTransition`] when the resource does not
/// exist or is not `trade_held`, [`ProfileError::InvalidInput`] when
/// `trade_id` is missing/malformed, and [`ProfileError::InvalidInput`] when
/// the `trade_id` input does not match the stored `trade_id`.
fn check_trade_unlock(
    current: Option<&StateProjection>,
    inputs: &BTreeMap<String, serde_json::Value>,
) -> Result<(), ProfileError> {
    let current = require_from(current, "trade.unlock", &[status::trade_held().to_owned()])?;
    let trade_id = input_str(inputs, "trade_id")?;
    let stored = state_str(current, "trade_id")?;
    if trade_id != stored {
        return Err(ProfileError::InvalidInput(format!(
            "trade_id `{trade_id}` does not match stored trade_id"
        )));
    }
    Ok(())
}

/// Validates `trade.settle`: transfers a `trade_held` asset to `to_owner`.
///
/// This is the ownership transfer for a settled trade. Requires `from_owner`
/// equal to the current owner, a `to_owner` input naming the new owner, and a
/// `trade_id` input matching the stored `trade_id`. After-state (computed by
/// the executor): owner = `to_owner`, status `active`, `trade_id` dropped.
///
/// The settle may optionally declare a value leg (asset-for-gold): the inputs
/// `value_resource`, `value_amount`, and `value_to_subject` describe the
/// fungible value exchanged for the asset. These are OPTIONAL for a pure
/// asset-for-asset settle; when any one is present, all three must be present
/// and coherent (missing any one fails closed). When present, `value_amount`
/// must parse as a canonical non-negative integer string and `value_resource` /
/// `value_to_subject` must be non-empty. The value leg does not change the
/// settle rule or consent gate; the batch-level balance arithmetic is enforced
/// by the executor's `validate_settle_batch`.
///
/// # Errors
///
/// Returns [`ProfileError::InvalidTransition`] when the resource does not
/// exist or is not `trade_held`, [`ProfileError::InvalidInput`] when
/// `to_owner`/`trade_id`/value-leg inputs are missing/malformed or the
/// `trade_id` does not match the stored value, and
/// [`ProfileError::OwnershipMismatch`] when `from_owner` is not the current
/// owner.
fn check_trade_settle(
    current: Option<&StateProjection>,
    inputs: &BTreeMap<String, serde_json::Value>,
) -> Result<(), ProfileError> {
    let current = require_from(current, "trade.settle", &[status::trade_held().to_owned()])?;
    let from_owner = input_str(inputs, "from_owner")?;
    input_str(inputs, "to_owner")?;
    let trade_id = input_str(inputs, "trade_id")?;
    check_value_leg(inputs)?;
    let owner = state_str(current, "owner")?;
    let stored = state_str(current, "trade_id")?;
    if from_owner != owner {
        return Err(ProfileError::OwnershipMismatch {
            expected: String::from(owner),
            actual: String::from(from_owner),
        });
    }
    if trade_id != stored {
        return Err(ProfileError::InvalidInput(format!(
            "trade_id `{trade_id}` does not match stored trade_id"
        )));
    }
    Ok(())
}

/// Validates an optional value-leg declaration on `trade.settle`.
///
/// The three value-leg inputs `value_resource`, `value_amount`, and
/// `value_to_subject` are optional together but mandatory once any one is
/// present: a partial declaration is incoherent and fails closed. When
/// present, `value_amount` must parse as a canonical non-negative integer
/// string and `value_resource` / `value_to_subject` must be non-empty.
///
/// # Errors
///
/// Returns [`ProfileError::InvalidInput`] when the declaration is partial, or
/// when a declared `value_amount` / `value_resource` / `value_to_subject` is
/// missing or malformed.
fn check_value_leg(inputs: &BTreeMap<String, serde_json::Value>) -> Result<(), ProfileError> {
    let declared = inputs.contains_key(keys::VALUE_RESOURCE)
        || inputs.contains_key(keys::VALUE_AMOUNT)
        || inputs.contains_key(keys::VALUE_TO_SUBJECT);
    if !declared {
        return Ok(());
    }
    // When the declaration is present, all three inputs must be present and
    // coherent; each helper fails closed on a missing or malformed member.
    input_str(inputs, keys::VALUE_RESOURCE)?;
    input_amount(inputs, keys::VALUE_AMOUNT)?;
    input_str(inputs, keys::VALUE_TO_SUBJECT)?;
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;
    use statechronicle_core::digest::ContentDigest;
    use statechronicle_domain::ids::{CommitId, EventId};
    use statechronicle_domain::resource::ResourceId;
    use statechronicle_domain::tenant::TenantId;

    fn asset(status: &Status, owner: &str) -> StateProjection {
        StateProjection {
            tenant_id: TenantId(String::from("tenant.test")),
            resource_id: ResourceId(String::from("asset:test")),
            state_type: StateType::UniqueAsset,
            version: 1,
            last_event_id: EventId::new(String::from("evt_01JZ8X2XRE5ZYW5V9R7VDQBSH4")).unwrap(),
            last_commit_id: CommitId::new(String::from("cmt_01JZ8X5HN3C4PXG5A9FGEWQF5W")).unwrap(),
            state_hash: ContentDigest::new([0u8; 32]),
            state: serde_json::json!({ "owner": owner, "status": status.as_str() }),
        }
    }

    fn inputs(entries: &[(&str, serde_json::Value)]) -> BTreeMap<String, serde_json::Value> {
        entries
            .iter()
            .map(|(key, value)| (String::from(*key), value.clone()))
            .collect()
    }

    /// A `trade_held` asset with a stored `trade_id`.
    fn held_asset(trade_id: &str) -> StateProjection {
        StateProjection {
            tenant_id: TenantId(String::from("tenant.test")),
            resource_id: ResourceId(String::from("asset:test")),
            state_type: StateType::UniqueAsset,
            version: 1,
            last_event_id: EventId::new(String::from("evt_01JZ8X2XRE5ZYW5V9R7VDQBSH4")).unwrap(),
            last_commit_id: CommitId::new(String::from("cmt_01JZ8X5HN3C4PXG5A9FGEWQF5W")).unwrap(),
            state_hash: ContentDigest::new([0u8; 32]),
            state: serde_json::json!({
                "owner": "alice",
                "status": status::trade_held().as_str(),
                "trade_id": trade_id,
            }),
        }
    }

    fn op(name: &str) -> Operation {
        Operation::new(String::from(name)).unwrap()
    }

    #[test]
    fn allow_list_is_complete() {
        let rules = UniqueAssetRules;
        assert_eq!(
            rules.allowed_operations(),
            &[
                op::asset_mint().to_owned(),
                op::asset_transfer().to_owned(),
                op::asset_burn().to_owned(),
                op::asset_lock().to_owned(),
                op::asset_unlock().to_owned(),
                op::asset_redeem().to_owned(),
                op::asset_list().to_owned(),
                op::asset_delist().to_owned(),
                op::asset_escrow().to_owned(),
                op::asset_release().to_owned(),
                op::asset_attach_content().to_owned(),
                op::asset_detach_content().to_owned(),
                op::asset_update_metadata().to_owned(),
                op::asset_restrict().to_owned(),
                op::asset_restore().to_owned(),
                op::trade_lock().to_owned(),
                op::trade_unlock().to_owned(),
                op::trade_settle().to_owned(),
            ]
        );
    }

    #[test]
    fn mint_requires_unborn_resource_and_to_owner() {
        let rules = UniqueAssetRules;
        assert!(
            rules
                .check(
                    &op("asset.mint"),
                    None,
                    &inputs(&[("to_owner", serde_json::json!("alice"))])
                )
                .is_ok()
        );
        assert!(matches!(
            rules.check(&op("asset.mint"), None, &BTreeMap::new()),
            Err(ProfileError::InvalidInput(_))
        ));
        let existing = asset(status::active(), "alice");
        assert!(matches!(
            rules.check(
                &op("asset.mint"),
                Some(&existing),
                &inputs(&[("to_owner", serde_json::json!("alice"))])
            ),
            Err(ProfileError::InvalidTransition { from, operation })
            if from == status::active().as_str() && operation == "asset.mint"
        ));
    }

    #[test]
    fn transfer_checks_ownership() {
        let rules = UniqueAssetRules;
        let active = asset(status::active(), "alice");
        assert!(
            rules
                .check(
                    &op("asset.transfer"),
                    Some(&active),
                    &inputs(&[
                        ("from_owner", serde_json::json!("alice")),
                        ("to_owner", serde_json::json!("bob")),
                    ])
                )
                .is_ok()
        );
        assert!(matches!(
            rules.check(
                &op("asset.transfer"),
                Some(&active),
                &inputs(&[
                    ("from_owner", serde_json::json!("mallory")),
                    ("to_owner", serde_json::json!("bob")),
                ])
            ),
            Err(ProfileError::OwnershipMismatch { expected, actual })
            if expected == "alice" && actual == "mallory"
        ));
        // Transfer from a non-active state is rejected.
        let locked = asset(status::locked(), "alice");
        assert!(matches!(
            rules.check(
                &op("asset.transfer"),
                Some(&locked),
                &inputs(&[
                    ("from_owner", serde_json::json!("alice")),
                    ("to_owner", serde_json::json!("bob")),
                ])
            ),
            Err(ProfileError::InvalidTransition { from, .. }) if from == status::locked().as_str()
        ));
    }

    #[test]
    fn burn_requires_owner_from_active() {
        let rules = UniqueAssetRules;
        let active = asset(status::active(), "alice");
        assert!(
            rules
                .check(
                    &op("asset.burn"),
                    Some(&active),
                    &inputs(&[("from_owner", serde_json::json!("alice"))])
                )
                .is_ok()
        );
        assert!(matches!(
            rules.check(
                &op("asset.burn"),
                Some(&active),
                &inputs(&[("from_owner", serde_json::json!("mallory"))])
            ),
            Err(ProfileError::OwnershipMismatch { .. })
        ));
        let locked = asset(status::locked(), "alice");
        assert!(matches!(
            rules.check(
                &op("asset.burn"),
                Some(&locked),
                &inputs(&[("from_owner", serde_json::json!("alice"))])
            ),
            Err(ProfileError::InvalidTransition { .. })
        ));
    }

    #[test]
    fn lock_unlock_list_delist_escrow_release_cycle() {
        let rules = UniqueAssetRules;
        let active = asset(status::active(), "alice");

        assert!(
            rules
                .check(&op("asset.lock"), Some(&active), &BTreeMap::new())
                .is_ok()
        );

        let locked = asset(status::locked(), "alice");
        assert!(
            rules
                .check(&op("asset.unlock"), Some(&locked), &BTreeMap::new())
                .is_ok()
        );

        let listed = asset(status::listed(), "alice");
        assert!(
            rules
                .check(&op("asset.delist"), Some(&listed), &BTreeMap::new())
                .is_ok()
        );
        assert!(
            rules
                .check(&op("asset.redeem"), Some(&listed), &BTreeMap::new())
                .is_ok()
        );

        let escrowed = asset(status::escrowed(), "alice");
        assert!(
            rules
                .check(&op("asset.release"), Some(&escrowed), &BTreeMap::new())
                .is_ok()
        );

        // unlock/delist/release require the matching source state.
        assert!(matches!(
            rules.check(&op("asset.unlock"), Some(&active), &BTreeMap::new()),
            Err(ProfileError::InvalidTransition { from, .. }) if from == status::active().as_str()
        ));
        assert!(matches!(
            rules.check(&op("asset.delist"), Some(&active), &BTreeMap::new()),
            Err(ProfileError::InvalidTransition { from, .. }) if from == status::active().as_str()
        ));
        assert!(matches!(
            rules.check(&op("asset.release"), Some(&active), &BTreeMap::new()),
            Err(ProfileError::InvalidTransition { from, .. }) if from == status::active().as_str()
        ));
    }

    #[test]
    fn restrict_restore_table() {
        let rules = UniqueAssetRules;
        for from in [
            status::active(),
            status::locked(),
            status::listed(),
            status::escrowed(),
        ] {
            let projection = asset(from, "alice");
            assert!(
                rules
                    .check(&op("asset.restrict"), Some(&projection), &BTreeMap::new())
                    .is_ok(),
                "restrict should be allowed from `{from}`"
            );
        }
        for from in [
            status::restricted(),
            status::quarantined(),
            status::unsupported(),
        ] {
            let projection = asset(from, "alice");
            assert!(
                rules
                    .check(&op("asset.restore"), Some(&projection), &BTreeMap::new())
                    .is_ok(),
                "restore should be allowed from `{from}`"
            );
        }
        // Restore from a terminal state is rejected.
        let burned = asset(status::burned(), "alice");
        assert!(matches!(
            rules.check(&op("asset.restore"), Some(&burned), &BTreeMap::new()),
            Err(ProfileError::InvalidTransition { from, .. }) if from == status::burned().as_str()
        ));
    }

    #[test]
    fn unknown_operations_are_rejected() {
        let rules = UniqueAssetRules;
        assert!(matches!(
            rules.check(&op("asset.teleport"), None, &BTreeMap::new()),
            Err(ProfileError::UnknownOperation(name)) if name == "asset.teleport"
        ));
        assert!(matches!(
            rules.check(&op("asset.hard_delete"), None, &BTreeMap::new()),
            Err(ProfileError::UnknownOperation(_))
        ));
    }

    #[test]
    fn operations_requiring_existence_fail_closed_on_none() {
        let rules = UniqueAssetRules;
        for name in [
            "asset.transfer",
            "asset.burn",
            "asset.lock",
            "asset.restore",
        ] {
            assert!(matches!(
                rules.check(&op(name), None, &BTreeMap::new()),
                Err(ProfileError::InvalidTransition { from, .. }) if from == "unborn"
            ));
        }
    }

    #[test]
    fn malformed_payload_fails_closed() {
        let rules = UniqueAssetRules;
        let broken = StateProjection {
            tenant_id: TenantId(String::from("tenant.test")),
            resource_id: ResourceId(String::from("asset:test")),
            state_type: StateType::UniqueAsset,
            version: 1,
            last_event_id: EventId::new(String::from("evt_01JZ8X2XRE5ZYW5V9R7VDQBSH4")).unwrap(),
            last_commit_id: CommitId::new(String::from("cmt_01JZ8X5HN3C4PXG5A9FGEWQF5W")).unwrap(),
            state_hash: ContentDigest::new([0u8; 32]),
            state: serde_json::json!({ "owner": "alice" }),
        };
        assert!(matches!(
            rules.check(&op("asset.lock"), Some(&broken), &BTreeMap::new()),
            Err(ProfileError::InvalidInput(_))
        ));
    }

    #[test]
    fn trade_lock_from_active_ok() {
        let rules = UniqueAssetRules;
        let active = asset(status::active(), "alice");
        assert!(
            rules
                .check(
                    &op("trade.lock"),
                    Some(&active),
                    &inputs(&[
                        ("from_owner", serde_json::json!("alice")),
                        ("trade_id", serde_json::json!("trade_001")),
                    ])
                )
                .is_ok()
        );
        // A non-owner cannot lock the asset into a trade.
        assert!(matches!(
            rules.check(
                &op("trade.lock"),
                Some(&active),
                &inputs(&[
                    ("from_owner", serde_json::json!("mallory")),
                    ("trade_id", serde_json::json!("trade_001")),
                ])
            ),
            Err(ProfileError::OwnershipMismatch { expected, actual })
            if expected == "alice" && actual == "mallory"
        ));
        // A trade lock must carry a trade_id.
        assert!(matches!(
            rules.check(
                &op("trade.lock"),
                Some(&active),
                &inputs(&[("from_owner", serde_json::json!("alice"))])
            ),
            Err(ProfileError::InvalidInput(_))
        ));
    }

    #[test]
    fn trade_lock_rejected_from_non_active() {
        let rules = UniqueAssetRules;
        for from in [status::locked(), status::listed(), status::trade_held()] {
            let projection = asset(from, "alice");
            assert!(
                matches!(
                    rules.check(
                        &op("trade.lock"),
                        Some(&projection),
                        &inputs(&[
                            ("from_owner", serde_json::json!("alice")),
                            ("trade_id", serde_json::json!("trade_001")),
                        ])
                    ),
                    Err(ProfileError::InvalidTransition { from: matched, .. })
                    if matched == from.as_str()
                ),
                "trade.lock should be rejected from `{from}`"
            );
        }
    }

    #[test]
    fn trade_unlock_requires_trade_held_and_matching_id() {
        let rules = UniqueAssetRules;
        let held = held_asset("trade_001");
        // Matching trade_id unlocks the held asset.
        assert!(
            rules
                .check(
                    &op("trade.unlock"),
                    Some(&held),
                    &inputs(&[("trade_id", serde_json::json!("trade_001"))])
                )
                .is_ok()
        );
        // A mismatched trade_id is rejected.
        assert!(matches!(
            rules.check(
                &op("trade.unlock"),
                Some(&held),
                &inputs(&[("trade_id", serde_json::json!("trade_999"))])
            ),
            Err(ProfileError::InvalidInput(_))
        ));
        // unlock from a non-trade_held state is rejected.
        let active = asset(status::active(), "alice");
        assert!(matches!(
            rules.check(
                &op("trade.unlock"),
                Some(&active),
                &inputs(&[("trade_id", serde_json::json!("trade_001"))])
            ),
            Err(ProfileError::InvalidTransition { from, .. }) if from == status::active().as_str()
        ));
    }

    #[test]
    fn trade_settle_requires_trade_held_owner_consent_and_to_owner() {
        let rules = UniqueAssetRules;
        let held = held_asset("trade_001");
        assert!(
            rules
                .check(
                    &op("trade.settle"),
                    Some(&held),
                    &inputs(&[
                        ("from_owner", serde_json::json!("alice")),
                        ("to_owner", serde_json::json!("bob")),
                        ("trade_id", serde_json::json!("trade_001")),
                    ])
                )
                .is_ok()
        );
        // settle requires the current owner's consent (from_owner == owner).
        assert!(matches!(
            rules.check(
                &op("trade.settle"),
                Some(&held),
                &inputs(&[
                    ("from_owner", serde_json::json!("mallory")),
                    ("to_owner", serde_json::json!("bob")),
                    ("trade_id", serde_json::json!("trade_001")),
                ])
            ),
            Err(ProfileError::OwnershipMismatch { expected, actual })
            if expected == "alice" && actual == "mallory"
        ));
        // settle requires a to_owner.
        assert!(matches!(
            rules.check(
                &op("trade.settle"),
                Some(&held),
                &inputs(&[
                    ("from_owner", serde_json::json!("alice")),
                    ("trade_id", serde_json::json!("trade_001")),
                ])
            ),
            Err(ProfileError::InvalidInput(_))
        ));
        // settle requires a matching trade_id.
        assert!(matches!(
            rules.check(
                &op("trade.settle"),
                Some(&held),
                &inputs(&[
                    ("from_owner", serde_json::json!("alice")),
                    ("to_owner", serde_json::json!("bob")),
                    ("trade_id", serde_json::json!("trade_999")),
                ])
            ),
            Err(ProfileError::InvalidInput(_))
        ));
    }

    #[test]
    fn trade_ops_require_authority_for_settle_only() {
        let rules = UniqueAssetRules;
        assert!(!rules.requires_authority(&op("trade.lock")));
        assert!(!rules.requires_authority(&op("trade.unlock")));
        assert!(rules.requires_authority(&op("trade.settle")));
    }

    #[test]
    fn trade_settle_with_full_value_leg_ok() {
        let rules = UniqueAssetRules;
        let held = held_asset("trade_001");
        assert!(
            rules
                .check(
                    &op("trade.settle"),
                    Some(&held),
                    &inputs(&[
                        ("from_owner", serde_json::json!("alice")),
                        ("to_owner", serde_json::json!("bob")),
                        ("trade_id", serde_json::json!("trade_001")),
                        ("value_resource", serde_json::json!("wallet:gold")),
                        ("value_amount", serde_json::json!("100")),
                        ("value_to_subject", serde_json::json!("alice")),
                    ])
                )
                .is_ok()
        );
        // A zero amount is a valid (non-negative) value declaration.
        assert!(
            rules
                .check(
                    &op("trade.settle"),
                    Some(&held),
                    &inputs(&[
                        ("from_owner", serde_json::json!("alice")),
                        ("to_owner", serde_json::json!("bob")),
                        ("trade_id", serde_json::json!("trade_001")),
                        ("value_resource", serde_json::json!("wallet:gold")),
                        ("value_amount", serde_json::json!("0")),
                        ("value_to_subject", serde_json::json!("alice")),
                    ])
                )
                .is_ok()
        );
    }

    #[test]
    fn trade_settle_partial_value_leg_fails_closed() {
        let rules = UniqueAssetRules;
        let held = held_asset("trade_001");
        // Missing value_amount -> InvalidInput.
        assert!(matches!(
            rules.check(
                &op("trade.settle"),
                Some(&held),
                &inputs(&[
                    ("from_owner", serde_json::json!("alice")),
                    ("to_owner", serde_json::json!("bob")),
                    ("trade_id", serde_json::json!("trade_001")),
                    ("value_resource", serde_json::json!("wallet:gold")),
                    ("value_to_subject", serde_json::json!("alice")),
                ])
            ),
            Err(ProfileError::InvalidInput(_))
        ));
        // Missing value_to_subject -> InvalidInput.
        assert!(matches!(
            rules.check(
                &op("trade.settle"),
                Some(&held),
                &inputs(&[
                    ("from_owner", serde_json::json!("alice")),
                    ("to_owner", serde_json::json!("bob")),
                    ("trade_id", serde_json::json!("trade_001")),
                    ("value_resource", serde_json::json!("wallet:gold")),
                    ("value_amount", serde_json::json!("100")),
                ])
            ),
            Err(ProfileError::InvalidInput(_))
        ));
        // Missing value_resource -> InvalidInput.
        assert!(matches!(
            rules.check(
                &op("trade.settle"),
                Some(&held),
                &inputs(&[
                    ("from_owner", serde_json::json!("alice")),
                    ("to_owner", serde_json::json!("bob")),
                    ("trade_id", serde_json::json!("trade_001")),
                    ("value_amount", serde_json::json!("100")),
                    ("value_to_subject", serde_json::json!("alice")),
                ])
            ),
            Err(ProfileError::InvalidInput(_))
        ));
        // A float-formatted value_amount fails closed (mirrors fungible_balance).
        assert!(matches!(
            rules.check(
                &op("trade.settle"),
                Some(&held),
                &inputs(&[
                    ("from_owner", serde_json::json!("alice")),
                    ("to_owner", serde_json::json!("bob")),
                    ("trade_id", serde_json::json!("trade_001")),
                    ("value_resource", serde_json::json!("wallet:gold")),
                    ("value_amount", serde_json::json!("1.0")),
                    ("value_to_subject", serde_json::json!("alice")),
                ])
            ),
            Err(ProfileError::FloatForbidden)
        ));
    }

    #[test]
    fn trade_settle_without_value_leg_still_ok() {
        let rules = UniqueAssetRules;
        let held = held_asset("trade_001");
        // A pure asset-for-asset settle carries no value-leg inputs and still
        // passes the settle rule unchanged.
        assert!(
            rules
                .check(
                    &op("trade.settle"),
                    Some(&held),
                    &inputs(&[
                        ("from_owner", serde_json::json!("alice")),
                        ("to_owner", serde_json::json!("bob")),
                        ("trade_id", serde_json::json!("trade_001")),
                    ])
                )
                .is_ok()
        );
    }
}
