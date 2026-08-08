//! Paid unique asset profile (protocol §20.3).
//!
//! An overlay over [`UniqueAssetRules`](crate::unique_asset::UniqueAssetRules)
//! for durable paid ownership. Paid assets refuse hard deletion without the
//! current owner's consent, require explicit owner authorization for transfers
//! by a non-owner, and guarantee that restriction preserves the `owner` field
//! in the projected payload (append-only, non-erasing exceptional states).

use std::collections::BTreeMap;
use std::sync::OnceLock;

use statechronicle_domain::intent::Operation;
use statechronicle_domain::state::StateProjection;
use statechronicle_domain::state_type::StateType;
use statechronicle_domain::status::Status;

use crate::error::ProfileError;
use crate::registry::{ProfileRules, input_str, require_current, require_from, state_str};
use crate::unique_asset::{UniqueAssetRules, status};

/// Typed exceptional statuses introduced by the paid overlay.
pub mod exceptional_status {
    use statechronicle_domain::status::Status;

    status_set! {
        /// The asset is under a legal hold.
        legal_hold => "legal_hold";
        /// The asset is locked against fraud.
        fraud_lock => "fraud_lock";
        /// The asset is restricted by marketplace policy.
        policy_restricted => "policy_restricted";
    }
}

/// The `authorized_by_owner` input key carrying owner consent.
const AUTHORIZED_BY_OWNER: &str = "authorized_by_owner";

/// Typed operation constants accepted by the paid unique asset profile.
pub mod op {
    use statechronicle_domain::intent::Operation;

    op_set! {
        /// Mints a new paid unique asset.
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
        /// Hard-deletes an asset with current-owner consent (terminal).
        asset_hard_delete => "asset.hard_delete";
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
        authority_required => [
            asset_transfer, asset_burn, asset_hard_delete, asset_restrict, asset_restore, trade_settle
        ];
    }
}

/// Exceptional statuses a paid asset may be restricted into.
///
/// The base unique asset's `restricted`, `quarantined`, and `unsupported`
/// statuses plus the paid overlay's `legal_hold`, `fraud_lock`, and
/// `policy_restricted` statuses. Restriction is append-only and non-erasing:
/// it preserves the `owner` field and never rewrites ownership.
fn exceptional_statuses() -> &'static [Status] {
    static ALL: OnceLock<Vec<Status>> = OnceLock::new();
    ALL.get_or_init(|| {
        vec![
            status::restricted().to_owned(),
            status::quarantined().to_owned(),
            status::unsupported().to_owned(),
            exceptional_status::legal_hold().to_owned(),
            exceptional_status::fraud_lock().to_owned(),
            exceptional_status::policy_restricted().to_owned(),
        ]
    })
}

/// Rule set for paid unique assets (protocol §20.3).
///
/// This is a distinct profile over the [`StateType::UniqueAsset`] state type.
/// It inherits the full unique asset transition table, then adds:
///
/// * `asset.hard_delete`: permitted only with `authorized_by_owner: true`
///   from the current owner (otherwise [`ProfileError::HardDeleteForbidden`]).
/// * `asset.transfer`: additionally requires `authorized_by_owner: true`
///   when the acting `actor` is not the current owner.
/// * `asset.restrict`: the target status may be any exceptional status
///   (`restricted`, `quarantine`, `legal_hold`, `fraud_lock`,
///   `policy_restricted`, `unsupported`) and the `owner` field is preserved:
///   any `owner` input must equal the current owner (append-only,
///   non-erasing).
/// * `asset.restore`: recovers from every exceptional status back to
///   `active`.
#[derive(Debug, Clone, Copy)]
pub struct PaidUniqueAssetRules;

impl ProfileRules for PaidUniqueAssetRules {
    fn state_type(&self) -> StateType {
        StateType::UniqueAsset
    }

    fn profile_id(&self) -> &'static str {
        "paid_unique_asset"
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
        if operation == op::asset_hard_delete() {
            return check_hard_delete(current, inputs);
        }
        if operation == op::asset_restore() {
            return check_paid_restore(current);
        }
        if !UniqueAssetRules.allowed_operations().contains(operation) {
            return Err(ProfileError::UnknownOperation(String::from(
                operation.as_str(),
            )));
        }
        UniqueAssetRules.check(operation, current, inputs)?;
        if operation == op::asset_transfer() {
            check_transfer_overlay(current, inputs)
        } else if operation == op::asset_restrict() {
            check_restrict_overlay(current, inputs)
        } else if operation == op::trade_lock() {
            check_trade_lock_overlay(current, inputs)
        } else if operation == op::trade_settle() {
            check_trade_settle_overlay(current, inputs)
        } else {
            Ok(())
        }
    }
}

/// Validates `asset.hard_delete`: current-owner consent is mandatory.
///
/// The acting `actor` must be the current owner and the
/// `authorized_by_owner` input must be exactly `true`.
///
/// # Errors
///
/// Returns [`ProfileError::InvalidTransition`] when the resource does not
/// exist, [`ProfileError::OwnershipMismatch`] when the `actor` is not the
/// current owner, and [`ProfileError::HardDeleteForbidden`] when
/// `authorized_by_owner` is missing or not `true`.
fn check_hard_delete(
    current: Option<&StateProjection>,
    inputs: &BTreeMap<String, serde_json::Value>,
) -> Result<(), ProfileError> {
    let current = require_current(current, "asset.hard_delete")?;
    let owner = state_str(current, "owner")?;
    let actor = input_str(inputs, "actor")?;
    if actor != owner {
        return Err(ProfileError::OwnershipMismatch {
            expected: String::from(owner),
            actual: String::from(actor),
        });
    }
    if inputs
        .get(AUTHORIZED_BY_OWNER)
        .and_then(serde_json::Value::as_bool)
        != Some(true)
    {
        return Err(ProfileError::HardDeleteForbidden);
    }
    Ok(())
}

/// Validates `asset.transfer` overlay: non-owner actors need consent.
///
/// The base table already enforces the `active` source state and
/// `from_owner == current owner`. When the acting `actor` is present and
/// differs from the current owner, `authorized_by_owner` must be `true`.
///
/// # Errors
///
/// Returns [`ProfileError::OwnershipMismatch`] when the `actor` is not the
/// current owner without consent, and
/// [`ProfileError::HardDeleteForbidden`] is never used here. A non-owner
/// without consent is reported as [`ProfileError::OwnershipMismatch`].
fn check_transfer_overlay(
    current: Option<&StateProjection>,
    inputs: &BTreeMap<String, serde_json::Value>,
) -> Result<(), ProfileError> {
    let current = require_from(current, "asset.transfer", &[status::active().to_owned()])?;
    let owner = state_str(current, "owner")?;
    let Some(actor) = inputs.get("actor").and_then(serde_json::Value::as_str) else {
        // No actor input means the caller provides no authorization signal;
        // the base table already enforced `from_owner == owner`.
        return Ok(());
    };
    if actor == owner {
        return Ok(());
    }
    if inputs
        .get(AUTHORIZED_BY_OWNER)
        .and_then(serde_json::Value::as_bool)
        != Some(true)
    {
        return Err(ProfileError::OwnershipMismatch {
            expected: String::from(owner),
            actual: String::from(actor),
        });
    }
    Ok(())
}

/// Validates `trade.lock` overlay: non-owner actors need consent.
///
/// The base table already enforces the `active` source state and
/// `from_owner == current owner`. When the acting `actor` is present and
/// differs from the current owner, `authorized_by_owner` must be `true`.
///
/// # Errors
///
/// Returns [`ProfileError::OwnershipMismatch`] when the `actor` is not the
/// current owner without consent.
fn check_trade_lock_overlay(
    current: Option<&StateProjection>,
    inputs: &BTreeMap<String, serde_json::Value>,
) -> Result<(), ProfileError> {
    check_owner_consent_overlay(
        current,
        inputs,
        "trade.lock",
        &[status::active().to_owned()],
    )
}

/// Validates `trade.settle` overlay: non-owner actors need consent.
///
/// The base table already enforces the `trade_held` source state and
/// `from_owner == current owner`. When the acting `actor` is present and
/// differs from the current owner, `authorized_by_owner` must be `true`.
///
/// # Errors
///
/// Returns [`ProfileError::OwnershipMismatch`] when the `actor` is not the
/// current owner without consent.
fn check_trade_settle_overlay(
    current: Option<&StateProjection>,
    inputs: &BTreeMap<String, serde_json::Value>,
) -> Result<(), ProfileError> {
    check_owner_consent_overlay(
        current,
        inputs,
        "trade.settle",
        &[status::trade_held().to_owned()],
    )
}

/// Shared owner-consent overlay for trade operations.
///
/// Mirrors [`check_transfer_overlay`]: when the acting `actor` is present and
/// differs from the current owner, `authorized_by_owner` must be `true`,
/// otherwise the operation fails closed with an ownership mismatch.
///
/// # Errors
///
/// Returns [`ProfileError::InvalidTransition`] when the resource's status is
/// not in `allowed_from`, and [`ProfileError::OwnershipMismatch`] when the
/// `actor` is not the current owner without consent.
fn check_owner_consent_overlay(
    current: Option<&StateProjection>,
    inputs: &BTreeMap<String, serde_json::Value>,
    operation: &str,
    allowed_from: &[Status],
) -> Result<(), ProfileError> {
    let current = require_from(current, operation, allowed_from)?;
    let owner = state_str(current, "owner")?;
    let Some(actor) = inputs.get("actor").and_then(serde_json::Value::as_str) else {
        // No actor input means the caller provides no authorization signal;
        // the base table already enforced `from_owner == owner`.
        return Ok(());
    };
    if actor == owner {
        return Ok(());
    }
    if inputs
        .get(AUTHORIZED_BY_OWNER)
        .and_then(serde_json::Value::as_bool)
        != Some(true)
    {
        return Err(ProfileError::OwnershipMismatch {
            expected: String::from(owner),
            actual: String::from(actor),
        });
    }
    Ok(())
}

/// Validates `asset.restrict` overlay: the owner field is preserved.
///
/// The base table already enforces the non-terminal source states. This
/// overlay additionally requires that any `owner` input equals the current
/// owner (append-only, non-erasing exceptional states) so a restriction can
/// never rewrite ownership, and that any `status` input names one of the
/// exceptional statuses.
///
/// # Errors
///
/// Returns [`ProfileError::OwnershipMismatch`] when an `owner` input differs
/// from the current owner, and [`ProfileError::InvalidInput`] when a `status`
/// input is not an exceptional status.
fn check_restrict_overlay(
    current: Option<&StateProjection>,
    inputs: &BTreeMap<String, serde_json::Value>,
) -> Result<(), ProfileError> {
    let current = require_from(
        current,
        "asset.restrict",
        &[
            status::active().to_owned(),
            status::locked().to_owned(),
            status::listed().to_owned(),
            status::escrowed().to_owned(),
            status::trade_held().to_owned(),
        ],
    )?;
    let owner = state_str(current, "owner")?;
    if let Some(provided) = inputs.get("owner").and_then(serde_json::Value::as_str)
        && provided != owner
    {
        return Err(ProfileError::OwnershipMismatch {
            expected: String::from(owner),
            actual: String::from(provided),
        });
    }
    if let Some(target) = inputs.get("status").and_then(serde_json::Value::as_str) {
        let Ok(target_status) = Status::try_from_str(target) else {
            return Err(ProfileError::InvalidInput(format!(
                "`status` `{target}` is not an exceptional status"
            )));
        };
        if !exceptional_statuses().contains(&target_status) {
            return Err(ProfileError::InvalidInput(format!(
                "`status` `{target}` is not an exceptional status"
            )));
        }
    }
    Ok(())
}

/// Validates `asset.restore` for the paid overlay.
///
/// In addition to the base table's source states (`restricted`, `quarantined`,
/// `unsupported`), a paid asset may be restored from the paid exceptional
/// statuses `legal_hold`, `fraud_lock`, and `policy_restricted`.
///
/// # Errors
///
/// Returns [`ProfileError::InvalidTransition`] when the resource does not
/// exist or its status is not recoverable.
fn check_paid_restore(current: Option<&StateProjection>) -> Result<(), ProfileError> {
    require_from(
        current,
        "asset.restore",
        &[
            status::restricted().to_owned(),
            status::quarantined().to_owned(),
            status::unsupported().to_owned(),
            exceptional_status::legal_hold().to_owned(),
            exceptional_status::fraud_lock().to_owned(),
            exceptional_status::policy_restricted().to_owned(),
        ],
    )?;
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
            resource_id: ResourceId(String::from("asset:paid_001")),
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

    /// A `trade_held` paid asset with a stored `trade_id`.
    fn held_asset(trade_id: &str) -> StateProjection {
        StateProjection {
            tenant_id: TenantId(String::from("tenant.test")),
            resource_id: ResourceId(String::from("asset:paid_001")),
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
    fn allow_list_includes_hard_delete() {
        let rules = PaidUniqueAssetRules;
        assert!(rules.allowed_operations().contains(op::asset_hard_delete()));
        assert!(rules.allowed_operations().contains(op::asset_transfer()));
        assert_eq!(rules.profile_id(), "paid_unique_asset");
        assert_eq!(rules.state_type(), StateType::UniqueAsset);
    }

    #[test]
    fn hard_delete_requires_owner_consent() {
        let rules = PaidUniqueAssetRules;
        let active = asset(status::active(), "alice");

        // Owner consent authorizes hard delete.
        assert!(
            rules
                .check(
                    &op("asset.hard_delete"),
                    Some(&active),
                    &inputs(&[
                        ("actor", serde_json::json!("alice")),
                        ("authorized_by_owner", serde_json::json!(true)),
                    ])
                )
                .is_ok()
        );

        // Without consent it is forbidden.
        assert!(matches!(
            rules.check(
                &op("asset.hard_delete"),
                Some(&active),
                &inputs(&[("actor", serde_json::json!("alice"))])
            ),
            Err(ProfileError::HardDeleteForbidden)
        ));
        assert!(matches!(
            rules.check(
                &op("asset.hard_delete"),
                Some(&active),
                &inputs(&[
                    ("actor", serde_json::json!("alice")),
                    ("authorized_by_owner", serde_json::json!(false)),
                ])
            ),
            Err(ProfileError::HardDeleteForbidden)
        ));

        // A non-owner cannot hard delete even with the consent flag.
        assert!(matches!(
            rules.check(
                &op("asset.hard_delete"),
                Some(&active),
                &inputs(&[
                    ("actor", serde_json::json!("mallory")),
                    ("authorized_by_owner", serde_json::json!(true)),
                ])
            ),
            Err(ProfileError::OwnershipMismatch { expected, actual })
            if expected == "alice" && actual == "mallory"
        ));

        // Hard delete on an unborn resource is rejected.
        assert!(matches!(
            rules.check(&op("asset.hard_delete"), None, &BTreeMap::new()),
            Err(ProfileError::InvalidTransition { from, .. }) if from == "unborn"
        ));
    }

    #[test]
    fn transfer_requires_authorized_by_owner_for_non_owner_actor() {
        let rules = PaidUniqueAssetRules;
        let active = asset(status::active(), "alice");

        // Owner acting: no consent needed.
        assert!(
            rules
                .check(
                    &op("asset.transfer"),
                    Some(&active),
                    &inputs(&[
                        ("actor", serde_json::json!("alice")),
                        ("from_owner", serde_json::json!("alice")),
                        ("to_owner", serde_json::json!("bob")),
                    ])
                )
                .is_ok()
        );

        // Non-owner actor without consent is rejected.
        assert!(matches!(
            rules.check(
                &op("asset.transfer"),
                Some(&active),
                &inputs(&[
                    ("actor", serde_json::json!("mallory")),
                    ("from_owner", serde_json::json!("alice")),
                    ("to_owner", serde_json::json!("bob")),
                ])
            ),
            Err(ProfileError::OwnershipMismatch { .. })
        ));

        // Non-owner actor with consent is allowed.
        assert!(
            rules
                .check(
                    &op("asset.transfer"),
                    Some(&active),
                    &inputs(&[
                        ("actor", serde_json::json!("mallory")),
                        ("authorized_by_owner", serde_json::json!(true)),
                        ("from_owner", serde_json::json!("alice")),
                        ("to_owner", serde_json::json!("bob")),
                    ])
                )
                .is_ok()
        );
    }

    #[test]
    fn restrict_preserves_owner_field() {
        let rules = PaidUniqueAssetRules;
        let active = asset(status::active(), "alice");

        // An owner-preserving restriction is accepted.
        assert!(
            rules
                .check(
                    &op("asset.restrict"),
                    Some(&active),
                    &inputs(&[("owner", serde_json::json!("alice"))])
                )
                .is_ok()
        );

        // Rewriting the owner is rejected (append-only, non-erasing).
        assert!(matches!(
            rules.check(
                &op("asset.restrict"),
                Some(&active),
                &inputs(&[("owner", serde_json::json!("mallory"))])
            ),
            Err(ProfileError::OwnershipMismatch { expected, actual })
            if expected == "alice" && actual == "mallory"
        ));
    }

    #[test]
    fn restrict_accepts_only_exceptional_target_statuses() {
        let rules = PaidUniqueAssetRules;
        let active = asset(status::active(), "alice");
        for target in [
            status::restricted(),
            status::quarantined(),
            status::unsupported(),
            exceptional_status::legal_hold(),
            exceptional_status::fraud_lock(),
            exceptional_status::policy_restricted(),
        ] {
            assert!(
                rules
                    .check(
                        &op("asset.restrict"),
                        Some(&active),
                        &inputs(&[("status", serde_json::json!(target))])
                    )
                    .is_ok(),
                "restrict should accept exceptional status `{target}`"
            );
        }
        assert!(matches!(
            rules.check(
                &op("asset.restrict"),
                Some(&active),
                &inputs(&[("status", serde_json::json!("burned"))])
            ),
            Err(ProfileError::InvalidInput(_))
        ));
    }

    #[test]
    fn restore_recovers_from_paid_exceptional_statuses() {
        let rules = PaidUniqueAssetRules;
        for from in [
            status::restricted(),
            status::quarantined(),
            status::unsupported(),
            exceptional_status::legal_hold(),
            exceptional_status::fraud_lock(),
            exceptional_status::policy_restricted(),
        ] {
            let projection = asset(from, "alice");
            assert!(
                rules
                    .check(&op("asset.restore"), Some(&projection), &BTreeMap::new())
                    .is_ok(),
                "restore should be allowed from `{from}`"
            );
        }
        let burned = asset(status::burned(), "alice");
        assert!(matches!(
            rules.check(&op("asset.restore"), Some(&burned), &BTreeMap::new()),
            Err(ProfileError::InvalidTransition { from, .. }) if from == status::burned().as_str()
        ));
    }

    #[test]
    fn delegates_base_table_and_rejects_unknown_ops() {
        let rules = PaidUniqueAssetRules;
        let active = asset(status::active(), "alice");
        // Base table still applies: lock from active is fine, unlock is not.
        assert!(
            rules
                .check(&op("asset.lock"), Some(&active), &BTreeMap::new())
                .is_ok()
        );
        assert!(matches!(
            rules.check(&op("asset.unlock"), Some(&active), &BTreeMap::new()),
            Err(ProfileError::InvalidTransition { from, .. }) if from == status::active().as_str()
        ));
        assert!(matches!(
            rules.check(&op("asset.teleport"), None, &BTreeMap::new()),
            Err(ProfileError::UnknownOperation(name)) if name == "asset.teleport"
        ));
    }

    #[test]
    fn trade_settle_requires_owner_consent_for_non_owner_actor() {
        let rules = PaidUniqueAssetRules;
        let held = held_asset("trade_001");

        // Owner acting: no consent needed.
        assert!(
            rules
                .check(
                    &op("trade.settle"),
                    Some(&held),
                    &inputs(&[
                        ("actor", serde_json::json!("alice")),
                        ("from_owner", serde_json::json!("alice")),
                        ("to_owner", serde_json::json!("bob")),
                        ("trade_id", serde_json::json!("trade_001")),
                    ])
                )
                .is_ok()
        );

        // Non-owner actor without consent is rejected.
        assert!(matches!(
            rules.check(
                &op("trade.settle"),
                Some(&held),
                &inputs(&[
                    ("actor", serde_json::json!("engine")),
                    ("from_owner", serde_json::json!("alice")),
                    ("to_owner", serde_json::json!("bob")),
                    ("trade_id", serde_json::json!("trade_001")),
                ])
            ),
            Err(ProfileError::OwnershipMismatch { .. })
        ));

        // Non-owner actor with owner consent is allowed.
        assert!(
            rules
                .check(
                    &op("trade.settle"),
                    Some(&held),
                    &inputs(&[
                        ("actor", serde_json::json!("engine")),
                        ("authorized_by_owner", serde_json::json!(true)),
                        ("from_owner", serde_json::json!("alice")),
                        ("to_owner", serde_json::json!("bob")),
                        ("trade_id", serde_json::json!("trade_001")),
                    ])
                )
                .is_ok()
        );
    }

    #[test]
    fn trade_lock_by_engine_with_owner_consent() {
        let rules = PaidUniqueAssetRules;
        let active = asset(status::active(), "alice");

        // Owner acting: no consent needed.
        assert!(
            rules
                .check(
                    &op("trade.lock"),
                    Some(&active),
                    &inputs(&[
                        ("actor", serde_json::json!("alice")),
                        ("from_owner", serde_json::json!("alice")),
                        ("trade_id", serde_json::json!("trade_001")),
                    ])
                )
                .is_ok()
        );

        // Engine (non-owner) actor without consent is rejected.
        assert!(matches!(
            rules.check(
                &op("trade.lock"),
                Some(&active),
                &inputs(&[
                    ("actor", serde_json::json!("engine")),
                    ("from_owner", serde_json::json!("alice")),
                    ("trade_id", serde_json::json!("trade_001")),
                ])
            ),
            Err(ProfileError::OwnershipMismatch { .. })
        ));

        // Engine actor with owner consent is allowed.
        assert!(
            rules
                .check(
                    &op("trade.lock"),
                    Some(&active),
                    &inputs(&[
                        ("actor", serde_json::json!("engine")),
                        ("authorized_by_owner", serde_json::json!(true)),
                        ("from_owner", serde_json::json!("alice")),
                        ("trade_id", serde_json::json!("trade_001")),
                    ])
                )
                .is_ok()
        );
    }

    #[test]
    fn trade_unlock_requires_matching_id_no_consent_needed() {
        let rules = PaidUniqueAssetRules;
        let held = held_asset("trade_001");

        // A matching trade_id unlocks; no actor consent required because the
        // asset only ever returns to its current owner.
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
    }
}
