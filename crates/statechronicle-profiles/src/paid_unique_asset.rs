//! Paid unique asset profile (protocol §20.3).
//!
//! An overlay over [`UniqueAssetRules`](crate::unique_asset::UniqueAssetRules)
//! for durable paid ownership. Paid assets refuse hard deletion without the
//! current owner's consent, require explicit owner authorization for transfers
//! by a non-owner, and guarantee that restriction preserves the `owner` field
//! in the projected payload (append-only, non-erasing exceptional states).

use std::collections::BTreeMap;

use statechronicle_domain::intent::Operation;
use statechronicle_domain::state::StateProjection;
use statechronicle_domain::state_type::StateType;

use crate::error::ProfileError;
use crate::registry::{ProfileRules, input_str, require_current, require_from, state_str};
use crate::unique_asset::{UniqueAssetRules, status};

/// Additional exceptional statuses introduced by the paid overlay.
pub(crate) mod exceptional_status {
    /// The asset is under a legal hold.
    pub(crate) const LEGAL_HOLD: &str = "legal_hold";
    /// The asset is locked against fraud.
    pub(crate) const FRAUD_LOCK: &str = "fraud_lock";
    /// The asset is restricted by marketplace policy.
    pub(crate) const POLICY_RESTRICTED: &str = "policy_restricted";
}

/// The `authorized_by_owner` input key carrying owner consent.
const AUTHORIZED_BY_OWNER: &str = "authorized_by_owner";

/// Exceptional statuses a paid asset may be restricted into.
///
/// The base unique asset's `restricted`, `quarantined`, and `unsupported`
/// statuses plus the paid overlay's `legal_hold`, `fraud_lock`, and
/// `policy_restricted` statuses. Restriction is append-only and non-erasing:
/// it preserves the `owner` field and never rewrites ownership.
const EXCEPTIONAL_STATUSES: &[&str] = &[
    status::RESTRICTED,
    status::QUARANTINED,
    status::UNSUPPORTED,
    exceptional_status::LEGAL_HOLD,
    exceptional_status::FRAUD_LOCK,
    exceptional_status::POLICY_RESTRICTED,
];

/// Rule set for paid unique assets (protocol §20.3).
///
/// This is a distinct profile over the [`StateType::UniqueAsset`] state type.
/// It inherits the full unique asset transition table, then adds:
///
/// * `asset.hard_delete` — permitted only with `authorized_by_owner: true`
///   from the current owner (otherwise [`ProfileError::HardDeleteForbidden`]).
/// * `asset.transfer` — additionally requires `authorized_by_owner: true`
///   when the acting `actor` is not the current owner.
/// * `asset.restrict` — the target status may be any exceptional status
///   (`restricted`, `quarantine`, `legal_hold`, `fraud_lock`,
///   `policy_restricted`, `unsupported`) and the `owner` field is preserved:
///   any `owner` input must equal the current owner (append-only,
///   non-erasing).
/// * `asset.restore` — recovers from every exceptional status back to
///   `active`.
#[derive(Debug, Clone, Copy)]
pub struct PaidUniqueAssetRules;

/// Operations accepted by the paid unique asset profile.
const OPERATIONS: &[&str] = &[
    "asset.mint",
    "asset.transfer",
    "asset.burn",
    "asset.lock",
    "asset.unlock",
    "asset.redeem",
    "asset.list",
    "asset.delist",
    "asset.escrow",
    "asset.release",
    "asset.attach_content",
    "asset.detach_content",
    "asset.update_metadata",
    "asset.restrict",
    "asset.restore",
    "asset.hard_delete",
];

/// Operations that MUST carry an authority binding (protocol §11.2,
/// ADR-006 §36 Q5 / deferral item 4).
///
/// A superset of the plain unique asset's authority-required set (ownership
/// transfer, terminal destruction) plus the paid-overlay's paid-restriction
/// and recovery paths (`asset.hard_delete`, `asset.restrict`, `asset.restore`).
const AUTHORITY_REQUIRED: &[&str] = &[
    "asset.transfer",
    "asset.burn",
    "asset.hard_delete",
    "asset.restrict",
    "asset.restore",
];

impl ProfileRules for PaidUniqueAssetRules {
    fn state_type(&self) -> StateType {
        StateType::UniqueAsset
    }

    fn profile_id(&self) -> &'static str {
        "paid_unique_asset"
    }

    fn allowed_operations(&self) -> &'static [&'static str] {
        OPERATIONS
    }

    fn requires_authority(&self, operation: &Operation) -> bool {
        AUTHORITY_REQUIRED.contains(&operation.as_str())
    }

    fn check(
        &self,
        operation: &Operation,
        current: Option<&StateProjection>,
        inputs: &BTreeMap<String, serde_json::Value>,
    ) -> Result<(), ProfileError> {
        let name = operation.as_str();
        if name == "asset.hard_delete" {
            return check_hard_delete(current, inputs);
        }
        if name == "asset.restore" {
            return check_paid_restore(current);
        }
        if !UniqueAssetRules.allowed_operations().contains(&name) {
            return Err(ProfileError::UnknownOperation(String::from(name)));
        }
        UniqueAssetRules.check(operation, current, inputs)?;
        match name {
            "asset.transfer" => check_transfer_overlay(current, inputs),
            "asset.restrict" => check_restrict_overlay(current, inputs),
            _ => Ok(()),
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
/// [`ProfileError::HardDeleteForbidden`] is never used here — a non-owner
/// without consent is reported as [`ProfileError::OwnershipMismatch`].
fn check_transfer_overlay(
    current: Option<&StateProjection>,
    inputs: &BTreeMap<String, serde_json::Value>,
) -> Result<(), ProfileError> {
    let current = require_from(current, "asset.transfer", &[status::ACTIVE])?;
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
            status::ACTIVE,
            status::LOCKED,
            status::LISTED,
            status::ESCROWED,
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
    if let Some(target) = inputs.get("status").and_then(serde_json::Value::as_str)
        && !EXCEPTIONAL_STATUSES.contains(&target)
    {
        return Err(ProfileError::InvalidInput(format!(
            "`status` `{target}` is not an exceptional status"
        )));
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
            status::RESTRICTED,
            status::QUARANTINED,
            status::UNSUPPORTED,
            exceptional_status::LEGAL_HOLD,
            exceptional_status::FRAUD_LOCK,
            exceptional_status::POLICY_RESTRICTED,
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

    fn asset(status: &str, owner: &str) -> StateProjection {
        StateProjection {
            tenant_id: TenantId(String::from("tenant.test")),
            resource_id: ResourceId(String::from("asset:paid_001")),
            state_type: StateType::UniqueAsset,
            version: 1,
            last_event_id: EventId::new(String::from("evt_01JZ8X2XRE5ZYW5V9R7VDQBSH4")).unwrap(),
            last_commit_id: CommitId::new(String::from("cmt_01JZ8X5HN3C4PXG5A9FGEWQF5W")).unwrap(),
            state_hash: ContentDigest::new([0u8; 32]),
            state: serde_json::json!({ "owner": owner, "status": status }),
        }
    }

    fn inputs(entries: &[(&str, serde_json::Value)]) -> BTreeMap<String, serde_json::Value> {
        entries
            .iter()
            .map(|(key, value)| (String::from(*key), value.clone()))
            .collect()
    }

    fn op(name: &str) -> Operation {
        Operation::new(String::from(name)).unwrap()
    }

    #[test]
    fn allow_list_includes_hard_delete() {
        let rules = PaidUniqueAssetRules;
        assert!(rules.allowed_operations().contains(&"asset.hard_delete"));
        assert!(rules.allowed_operations().contains(&"asset.transfer"));
        assert_eq!(rules.profile_id(), "paid_unique_asset");
        assert_eq!(rules.state_type(), StateType::UniqueAsset);
    }

    #[test]
    fn hard_delete_requires_owner_consent() {
        let rules = PaidUniqueAssetRules;
        let active = asset(status::ACTIVE, "alice");

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
        let active = asset(status::ACTIVE, "alice");

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
        let active = asset(status::ACTIVE, "alice");

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
        let active = asset(status::ACTIVE, "alice");
        for target in [
            "restricted",
            "quarantined",
            "unsupported",
            exceptional_status::LEGAL_HOLD,
            exceptional_status::FRAUD_LOCK,
            exceptional_status::POLICY_RESTRICTED,
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
            status::RESTRICTED,
            status::QUARANTINED,
            status::UNSUPPORTED,
            exceptional_status::LEGAL_HOLD,
            exceptional_status::FRAUD_LOCK,
            exceptional_status::POLICY_RESTRICTED,
        ] {
            let projection = asset(from, "alice");
            assert!(
                rules
                    .check(&op("asset.restore"), Some(&projection), &BTreeMap::new())
                    .is_ok(),
                "restore should be allowed from `{from}`"
            );
        }
        let burned = asset(status::BURNED, "alice");
        assert!(matches!(
            rules.check(&op("asset.restore"), Some(&burned), &BTreeMap::new()),
            Err(ProfileError::InvalidTransition { from, .. }) if from == status::BURNED
        ));
    }

    #[test]
    fn delegates_base_table_and_rejects_unknown_ops() {
        let rules = PaidUniqueAssetRules;
        let active = asset(status::ACTIVE, "alice");
        // Base table still applies: lock from active is fine, unlock is not.
        assert!(
            rules
                .check(&op("asset.lock"), Some(&active), &BTreeMap::new())
                .is_ok()
        );
        assert!(matches!(
            rules.check(&op("asset.unlock"), Some(&active), &BTreeMap::new()),
            Err(ProfileError::InvalidTransition { from, .. }) if from == status::ACTIVE
        ));
        assert!(matches!(
            rules.check(&op("asset.teleport"), None, &BTreeMap::new()),
            Err(ProfileError::UnknownOperation(name)) if name == "asset.teleport"
        ));
    }
}
