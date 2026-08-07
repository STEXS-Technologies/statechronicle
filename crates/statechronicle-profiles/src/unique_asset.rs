//! Unique asset profile.
//!
//! Singleton resources with ownership rules (protocol §20.1). A unique asset
//! carries an `owner` and a `status` in its projected payload and moves through
//! an explicit transition table. Hard delete is never permitted: `burn` is the
//! only terminal destructive transition and it requires owner authorization.

use std::collections::BTreeMap;

use statechronicle_domain::intent::Operation;
use statechronicle_domain::state::StateProjection;
use statechronicle_domain::state_type::StateType;

use crate::error::ProfileError;
use crate::registry::{ProfileRules, input_str, require_from, state_str};

/// Wire status names for a unique asset.
pub(crate) mod status {
    /// The asset is held by its owner and may transition normally.
    pub(crate) const ACTIVE: &str = "active";
    /// The asset is locked and cannot be transferred, listed, or burned.
    pub(crate) const LOCKED: &str = "locked";
    /// The asset is listed for sale.
    pub(crate) const LISTED: &str = "listed";
    /// The asset is held in escrow pending settlement.
    pub(crate) const ESCROWED: &str = "escrowed";
    /// The asset was redeemed (for example a voucher or ticket consumed).
    #[allow(dead_code)] // wire-format constant referenced by transition tests
    pub(crate) const REDEEMED: &str = "redeemed";
    /// The asset was burned (terminal).
    #[allow(dead_code)] // wire-format constant referenced by transition tests
    pub(crate) const BURNED: &str = "burned";
    /// The asset is restricted by policy.
    pub(crate) const RESTRICTED: &str = "restricted";
    /// The asset is under quarantine.
    pub(crate) const QUARANTINED: &str = "quarantined";
    /// The asset is in an unsupported state.
    pub(crate) const UNSUPPORTED: &str = "unsupported";
    /// The asset is tombstoned (soft-deleted, terminal).
    #[allow(dead_code)] // wire-format constant referenced by transition tests
    pub(crate) const TOMBSTONED: &str = "tombstoned";
}

/// Operations accepted by the unique asset profile.
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
];

/// Operations that MUST carry an authority binding (protocol §11.2,
/// ADR-006 §36 Q5 / deferral item 4).
///
/// Ownership-transfer and terminal-destruction paths require a TrustGrant
/// authority proof; the profile's own rules then gate consent and state.
const AUTHORITY_REQUIRED: &[&str] = &["asset.transfer", "asset.burn"];

/// Rule set for [`StateType::UniqueAsset`] (protocol §20.1).
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
/// | `asset.restrict` | `active`/`locked`/`listed`/`escrowed` | `restricted` |
/// | `asset.restore` | `restricted`/`quarantined`/`unsupported` | `active` |
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
        if !OPERATIONS.contains(&operation.as_str()) {
            return Err(ProfileError::UnknownOperation(String::from(
                operation.as_str(),
            )));
        }
        match operation.as_str() {
            "asset.mint" => check_mint(current, inputs),
            "asset.transfer" => check_transfer(current, inputs),
            "asset.burn" => check_burn(current, inputs),
            "asset.lock" => single_from(current, "asset.lock", &[status::ACTIVE]),
            "asset.unlock" => single_from(current, "asset.unlock", &[status::LOCKED]),
            "asset.list" => single_from(current, "asset.list", &[status::ACTIVE]),
            "asset.delist" => single_from(current, "asset.delist", &[status::LISTED]),
            "asset.escrow" => single_from(current, "asset.escrow", &[status::ACTIVE]),
            "asset.release" => single_from(current, "asset.release", &[status::ESCROWED]),
            "asset.redeem" => single_from(current, "asset.redeem", &[status::LISTED]),
            "asset.attach_content" => {
                single_from(current, "asset.attach_content", &[status::ACTIVE])
            }
            "asset.detach_content" => {
                single_from(current, "asset.detach_content", &[status::ACTIVE])
            }
            "asset.update_metadata" => {
                single_from(current, "asset.update_metadata", &[status::ACTIVE])
            }
            "asset.restrict" => single_from(
                current,
                "asset.restrict",
                &[
                    status::ACTIVE,
                    status::LOCKED,
                    status::LISTED,
                    status::ESCROWED,
                ],
            ),
            "asset.restore" => single_from(
                current,
                "asset.restore",
                &[status::RESTRICTED, status::QUARANTINED, status::UNSUPPORTED],
            ),
            _ => Err(ProfileError::UnknownOperation(String::from(
                operation.as_str(),
            ))),
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
    allowed_from: &[&str],
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
    let current = require_from(current, "asset.transfer", &[status::ACTIVE])?;
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
    let current = require_from(current, "asset.burn", &[status::ACTIVE])?;
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
            resource_id: ResourceId(String::from("asset:test")),
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
    fn allow_list_is_complete() {
        let rules = UniqueAssetRules;
        assert_eq!(
            rules.allowed_operations(),
            &[
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
            ][..]
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
        let existing = asset(status::ACTIVE, "alice");
        assert!(matches!(
            rules.check(
                &op("asset.mint"),
                Some(&existing),
                &inputs(&[("to_owner", serde_json::json!("alice"))])
            ),
            Err(ProfileError::InvalidTransition { from, operation })
            if from == status::ACTIVE && operation == "asset.mint"
        ));
    }

    #[test]
    fn transfer_checks_ownership() {
        let rules = UniqueAssetRules;
        let active = asset(status::ACTIVE, "alice");
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
        let locked = asset(status::LOCKED, "alice");
        assert!(matches!(
            rules.check(
                &op("asset.transfer"),
                Some(&locked),
                &inputs(&[
                    ("from_owner", serde_json::json!("alice")),
                    ("to_owner", serde_json::json!("bob")),
                ])
            ),
            Err(ProfileError::InvalidTransition { from, .. }) if from == status::LOCKED
        ));
    }

    #[test]
    fn burn_requires_owner_from_active() {
        let rules = UniqueAssetRules;
        let active = asset(status::ACTIVE, "alice");
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
        let locked = asset(status::LOCKED, "alice");
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
        let active = asset(status::ACTIVE, "alice");

        assert!(
            rules
                .check(&op("asset.lock"), Some(&active), &BTreeMap::new())
                .is_ok()
        );

        let locked = asset(status::LOCKED, "alice");
        assert!(
            rules
                .check(&op("asset.unlock"), Some(&locked), &BTreeMap::new())
                .is_ok()
        );

        let listed = asset(status::LISTED, "alice");
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

        let escrowed = asset(status::ESCROWED, "alice");
        assert!(
            rules
                .check(&op("asset.release"), Some(&escrowed), &BTreeMap::new())
                .is_ok()
        );

        // unlock/delist/release require the matching source state.
        assert!(matches!(
            rules.check(&op("asset.unlock"), Some(&active), &BTreeMap::new()),
            Err(ProfileError::InvalidTransition { from, .. }) if from == status::ACTIVE
        ));
        assert!(matches!(
            rules.check(&op("asset.delist"), Some(&active), &BTreeMap::new()),
            Err(ProfileError::InvalidTransition { from, .. }) if from == status::ACTIVE
        ));
        assert!(matches!(
            rules.check(&op("asset.release"), Some(&active), &BTreeMap::new()),
            Err(ProfileError::InvalidTransition { from, .. }) if from == status::ACTIVE
        ));
    }

    #[test]
    fn restrict_restore_table() {
        let rules = UniqueAssetRules;
        for from in [
            status::ACTIVE,
            status::LOCKED,
            status::LISTED,
            status::ESCROWED,
        ] {
            let projection = asset(from, "alice");
            assert!(
                rules
                    .check(&op("asset.restrict"), Some(&projection), &BTreeMap::new())
                    .is_ok(),
                "restrict should be allowed from `{from}`"
            );
        }
        for from in [status::RESTRICTED, status::QUARANTINED, status::UNSUPPORTED] {
            let projection = asset(from, "alice");
            assert!(
                rules
                    .check(&op("asset.restore"), Some(&projection), &BTreeMap::new())
                    .is_ok(),
                "restore should be allowed from `{from}`"
            );
        }
        // Restore from a terminal state is rejected.
        let burned = asset(status::BURNED, "alice");
        assert!(matches!(
            rules.check(&op("asset.restore"), Some(&burned), &BTreeMap::new()),
            Err(ProfileError::InvalidTransition { from, .. }) if from == status::BURNED
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
}
