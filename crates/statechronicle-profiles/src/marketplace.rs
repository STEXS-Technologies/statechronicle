//! Marketplace profile (protocol §20.9).
//!
//! Listings and escrow used by atomic purchase settlement. A listing carries
//! a `seller` and a `status` in its projected payload; an escrow carries a
//! `buyer`, a `seller`, and a `status`.
//!
//! **Atomic multi-resource requirement.** A purchase is not a single
//! transition: it atomically advances a listing to `sold` and the matching
//! escrow to `released` (and applies the settlement to the fungible balances
//! involved) in one commit. The protocol therefore never lets a `listing.buy`
//! or `escrow.release` commit alone. Settlement must land as a multi-resource
//! transaction covering every affected resource (§20.9).

use std::collections::BTreeMap;

use statechronicle_domain::intent::Operation;
use statechronicle_domain::state::StateProjection;
use statechronicle_domain::state_type::StateType;

use crate::error::ProfileError;
use crate::registry::{ProfileRules, input_str, require_from, require_unborn};

/// Wire status names for a listing.
mod listing_status {
    /// The listing is live and may be bought, cancelled, or expired.
    pub(super) const LISTED: &str = "listed";
    /// The listing was cancelled by the seller (terminal).
    #[allow(dead_code)] // wire-format constant referenced by transition tests
    pub(super) const CANCELLED: &str = "cancelled";
    /// The listing was sold (terminal).
    #[allow(dead_code)] // wire-format constant referenced by transition tests
    pub(super) const SOLD: &str = "sold";
    /// The listing expired (terminal).
    #[allow(dead_code)] // wire-format constant referenced by transition tests
    pub(super) const EXPIRED: &str = "expired";
}

/// Wire status names for an escrow.
mod escrow_status {
    /// The escrow is holding the funds.
    pub(super) const LOCKED: &str = "locked";
    /// The escrow was released to the seller (terminal).
    #[allow(dead_code)] // wire-format constant referenced by transition tests
    pub(super) const RELEASED: &str = "released";
    /// The escrow was refunded to the buyer (terminal).
    #[allow(dead_code)] // wire-format constant referenced by transition tests
    pub(super) const REFUNDED: &str = "refunded";
}

/// Operations accepted by the listing profile.
const LISTING_OPERATIONS: &[&str] = &[
    "listing.create",
    "listing.cancel",
    "listing.buy",
    "listing.expire",
];

/// Operations accepted by the escrow profile.
const ESCROW_OPERATIONS: &[&str] = &["escrow.lock", "escrow.release", "escrow.refund"];

/// Rule set for [`StateType::Listing`] (protocol §20.9).
///
/// # Transition table
///
/// | operation | from | to |
/// |---|---|---|
/// | `listing.create` | *(no prior state)* | `listed` |
/// | `listing.cancel` | `listed` | `cancelled` |
/// | `listing.buy` | `listed` | `sold` |
/// | `listing.expire` | `listed` | `expired` |
///
/// `cancelled`, `sold`, and `expired` are terminal.
#[derive(Debug, Clone, Copy)]
pub struct ListingRules;

/// Rule set for [`StateType::Escrow`] (protocol §20.9).
///
/// # Transition table
///
/// | operation | from | to |
/// |---|---|---|
/// | `escrow.lock` | *(no prior state)* | `locked` |
/// | `escrow.release` | `locked` | `released` |
/// | `escrow.refund` | `locked` | `refunded` |
///
/// `released` and `refunded` are terminal.
#[derive(Debug, Clone, Copy)]
pub struct EscrowRules;

impl ProfileRules for ListingRules {
    fn state_type(&self) -> StateType {
        StateType::Listing
    }

    fn profile_id(&self) -> &'static str {
        "listing"
    }

    fn allowed_operations(&self) -> &'static [&'static str] {
        LISTING_OPERATIONS
    }

    fn check(
        &self,
        operation: &Operation,
        current: Option<&StateProjection>,
        inputs: &BTreeMap<String, serde_json::Value>,
    ) -> Result<(), ProfileError> {
        if !LISTING_OPERATIONS.contains(&operation.as_str()) {
            return Err(ProfileError::UnknownOperation(String::from(
                operation.as_str(),
            )));
        }
        match operation.as_str() {
            "listing.create" => check_listing_create(current, inputs),
            "listing.cancel" => single_from(current, "listing.cancel", &[listing_status::LISTED]),
            "listing.buy" => check_listing_buy(current, inputs),
            "listing.expire" => single_from(current, "listing.expire", &[listing_status::LISTED]),
            _ => Err(ProfileError::UnknownOperation(String::from(
                operation.as_str(),
            ))),
        }
    }
}

impl ProfileRules for EscrowRules {
    fn state_type(&self) -> StateType {
        StateType::Escrow
    }

    fn profile_id(&self) -> &'static str {
        "escrow"
    }

    fn allowed_operations(&self) -> &'static [&'static str] {
        ESCROW_OPERATIONS
    }

    fn check(
        &self,
        operation: &Operation,
        current: Option<&StateProjection>,
        inputs: &BTreeMap<String, serde_json::Value>,
    ) -> Result<(), ProfileError> {
        if !ESCROW_OPERATIONS.contains(&operation.as_str()) {
            return Err(ProfileError::UnknownOperation(String::from(
                operation.as_str(),
            )));
        }
        match operation.as_str() {
            "escrow.lock" => check_escrow_lock(current, inputs),
            "escrow.release" => single_from(current, "escrow.release", &[escrow_status::LOCKED]),
            "escrow.refund" => single_from(current, "escrow.refund", &[escrow_status::LOCKED]),
            _ => Err(ProfileError::UnknownOperation(String::from(
                operation.as_str(),
            ))),
        }
    }
}

/// Validates a single-source transition with no input requirements.
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

/// Validates `listing.create`.
///
/// # Errors
///
/// Returns [`ProfileError::InvalidTransition`] when a listing already exists,
/// and [`ProfileError::InvalidInput`] when the `seller` input is
/// missing/malformed.
fn check_listing_create(
    current: Option<&StateProjection>,
    inputs: &BTreeMap<String, serde_json::Value>,
) -> Result<(), ProfileError> {
    require_unborn(current, "listing.create")?;
    input_str(inputs, "seller")?;
    Ok(())
}

/// Validates `listing.buy`.
///
/// # Errors
///
/// Returns [`ProfileError::InvalidTransition`] when the listing does not
/// exist or is not `listed`, and [`ProfileError::InvalidInput`] when the
/// `buyer` input is missing/malformed.
fn check_listing_buy(
    current: Option<&StateProjection>,
    inputs: &BTreeMap<String, serde_json::Value>,
) -> Result<(), ProfileError> {
    require_from(current, "listing.buy", &[listing_status::LISTED])?;
    input_str(inputs, "buyer")?;
    Ok(())
}

/// Validates `escrow.lock`.
///
/// # Errors
///
/// Returns [`ProfileError::InvalidTransition`] when an escrow already exists,
/// and [`ProfileError::InvalidInput`] when the `buyer` or `seller` input is
/// missing/malformed.
fn check_escrow_lock(
    current: Option<&StateProjection>,
    inputs: &BTreeMap<String, serde_json::Value>,
) -> Result<(), ProfileError> {
    require_unborn(current, "escrow.lock")?;
    input_str(inputs, "buyer")?;
    input_str(inputs, "seller")?;
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

    fn projection(state_type: StateType, status: &str, fields: &[(&str, &str)]) -> StateProjection {
        let mut state = serde_json::json!({ "status": status });
        let object = state.as_object_mut().unwrap();
        for (key, value) in fields {
            object.insert(
                String::from(*key),
                serde_json::Value::String(String::from(*value)),
            );
        }
        StateProjection {
            tenant_id: TenantId(String::from("tenant.test")),
            resource_id: ResourceId(String::from("listing:001")),
            state_type,
            version: 1,
            last_event_id: EventId::new(String::from("evt_01JZ8X2XRE5ZYW5V9R7VDQBSH4")).unwrap(),
            last_commit_id: CommitId::new(String::from("cmt_01JZ8X5HN3C4PXG5A9FGEWQF5W")).unwrap(),
            state_hash: ContentDigest::new([0u8; 32]),
            state,
        }
    }

    fn listing(status: &str) -> StateProjection {
        projection(StateType::Listing, status, &[("seller", "alice")])
    }

    fn escrow(status: &str) -> StateProjection {
        projection(
            StateType::Escrow,
            status,
            &[("buyer", "bob"), ("seller", "alice")],
        )
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
    fn listing_allow_list_is_complete() {
        assert_eq!(
            ListingRules.allowed_operations(),
            &[
                "listing.create",
                "listing.cancel",
                "listing.buy",
                "listing.expire"
            ][..]
        );
        assert_eq!(
            EscrowRules.allowed_operations(),
            &["escrow.lock", "escrow.release", "escrow.refund"][..]
        );
    }

    #[test]
    fn listing_create_cancel_buy_expire() {
        let rules = ListingRules;
        assert!(
            rules
                .check(
                    &op("listing.create"),
                    None,
                    &inputs(&[("seller", serde_json::json!("alice"))])
                )
                .is_ok()
        );
        let listed = listing(listing_status::LISTED);
        assert!(
            rules
                .check(&op("listing.cancel"), Some(&listed), &BTreeMap::new())
                .is_ok()
        );
        assert!(
            rules
                .check(&op("listing.expire"), Some(&listed), &BTreeMap::new())
                .is_ok()
        );
        assert!(
            rules
                .check(
                    &op("listing.buy"),
                    Some(&listed),
                    &inputs(&[("buyer", serde_json::json!("bob"))])
                )
                .is_ok()
        );

        // Terminal states are locked.
        let sold = listing(listing_status::SOLD);
        assert!(matches!(
            rules.check(&op("listing.cancel"), Some(&sold), &BTreeMap::new()),
            Err(ProfileError::InvalidTransition { from, .. }) if from == listing_status::SOLD
        ));
        // Missing seller / buyer inputs are rejected.
        assert!(matches!(
            rules.check(&op("listing.create"), None, &BTreeMap::new()),
            Err(ProfileError::InvalidInput(_))
        ));
        assert!(matches!(
            rules.check(&op("listing.buy"), Some(&listed), &BTreeMap::new()),
            Err(ProfileError::InvalidInput(_))
        ));
    }

    #[test]
    fn escrow_lock_release_refund() {
        let rules = EscrowRules;
        assert!(
            rules
                .check(
                    &op("escrow.lock"),
                    None,
                    &inputs(&[
                        ("buyer", serde_json::json!("bob")),
                        ("seller", serde_json::json!("alice")),
                    ])
                )
                .is_ok()
        );
        let locked = escrow(escrow_status::LOCKED);
        assert!(
            rules
                .check(&op("escrow.release"), Some(&locked), &BTreeMap::new())
                .is_ok()
        );
        assert!(
            rules
                .check(&op("escrow.refund"), Some(&locked), &BTreeMap::new())
                .is_ok()
        );

        // Terminal states are locked.
        let released = escrow(escrow_status::RELEASED);
        assert!(matches!(
            rules.check(&op("escrow.refund"), Some(&released), &BTreeMap::new()),
            Err(ProfileError::InvalidTransition { from, .. }) if from == escrow_status::RELEASED
        ));
        assert!(matches!(
            rules.check(&op("escrow.lock"), None, &BTreeMap::new()),
            Err(ProfileError::InvalidInput(_))
        ));
    }

    #[test]
    fn cross_profile_operations_are_unknown() {
        assert!(matches!(
            ListingRules.check(&op("escrow.lock"), None, &BTreeMap::new()),
            Err(ProfileError::UnknownOperation(name)) if name == "escrow.lock"
        ));
        assert!(matches!(
            EscrowRules.check(&op("listing.create"), None, &BTreeMap::new()),
            Err(ProfileError::UnknownOperation(name)) if name == "listing.create"
        ));
    }
}
