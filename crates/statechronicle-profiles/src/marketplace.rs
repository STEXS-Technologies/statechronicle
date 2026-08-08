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
use statechronicle_domain::status::Status;

use crate::error::ProfileError;
use crate::registry::{ProfileRules, input_str, require_from, require_unborn};

/// Typed wire status names for a listing.
pub mod listing_status {
    use statechronicle_domain::status::Status;

    status_set! {
        /// The listing is live and may be bought, cancelled, or expired.
        listed => "listed";
        /// The listing was cancelled by the seller (terminal).
        cancelled => "cancelled";
        /// The listing was sold (terminal).
        sold => "sold";
        /// The listing expired (terminal).
        expired => "expired";
    }
}

/// Typed wire status names for an escrow.
pub mod escrow_status {
    use statechronicle_domain::status::Status;

    status_set! {
        /// The escrow is holding the funds.
        locked => "locked";
        /// The escrow was released to the seller (terminal).
        released => "released";
        /// The escrow was refunded to the buyer (terminal).
        refunded => "refunded";
    }
}

/// Typed operation constants accepted by the marketplace profiles.
pub mod op {
    use statechronicle_domain::intent::Operation;

    op_set! {
        /// Creates a new listing.
        listing_create => "listing.create";
        /// Cancels a listed listing (terminal).
        listing_cancel => "listing.cancel";
        /// Buys a listed listing (terminal).
        listing_buy => "listing.buy";
        /// Expires a listed listing (terminal).
        listing_expire => "listing.expire";
        /// Locks a new escrow.
        escrow_lock => "escrow.lock";
        /// Releases a locked escrow (terminal).
        escrow_release => "escrow.release";
        /// Refunds a locked escrow (terminal).
        escrow_refund => "escrow.refund";
    }

    op_slice! {
        /// All operations accepted by the listing profile.
        listing_operations => [ listing_create, listing_cancel, listing_buy, listing_expire ];
        /// All operations accepted by the escrow profile.
        escrow_operations => [ escrow_lock, escrow_release, escrow_refund ];
    }
}

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

    fn allowed_operations(&self) -> &'static [Operation] {
        op::listing_operations()
    }

    fn check(
        &self,
        operation: &Operation,
        current: Option<&StateProjection>,
        inputs: &BTreeMap<String, serde_json::Value>,
    ) -> Result<(), ProfileError> {
        if !op::listing_operations().contains(operation) {
            return Err(ProfileError::UnknownOperation(String::from(
                operation.as_str(),
            )));
        }
        if operation == op::listing_create() {
            check_listing_create(current, inputs)
        } else if operation == op::listing_cancel() {
            single_from(
                current,
                "listing.cancel",
                &[listing_status::listed().to_owned()],
            )
        } else if operation == op::listing_buy() {
            check_listing_buy(current, inputs)
        } else if operation == op::listing_expire() {
            single_from(
                current,
                "listing.expire",
                &[listing_status::listed().to_owned()],
            )
        } else {
            Err(ProfileError::UnknownOperation(String::from(
                operation.as_str(),
            )))
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

    fn allowed_operations(&self) -> &'static [Operation] {
        op::escrow_operations()
    }

    fn check(
        &self,
        operation: &Operation,
        current: Option<&StateProjection>,
        inputs: &BTreeMap<String, serde_json::Value>,
    ) -> Result<(), ProfileError> {
        if !op::escrow_operations().contains(operation) {
            return Err(ProfileError::UnknownOperation(String::from(
                operation.as_str(),
            )));
        }
        if operation == op::escrow_lock() {
            check_escrow_lock(current, inputs)
        } else if operation == op::escrow_release() {
            single_from(
                current,
                "escrow.release",
                &[escrow_status::locked().to_owned()],
            )
        } else if operation == op::escrow_refund() {
            single_from(
                current,
                "escrow.refund",
                &[escrow_status::locked().to_owned()],
            )
        } else {
            Err(ProfileError::UnknownOperation(String::from(
                operation.as_str(),
            )))
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
    allowed_from: &[Status],
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
    require_from(
        current,
        "listing.buy",
        &[listing_status::listed().to_owned()],
    )?;
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

    fn listing(status: &Status) -> StateProjection {
        projection(StateType::Listing, status.as_str(), &[("seller", "alice")])
    }

    fn escrow(status: &Status) -> StateProjection {
        projection(
            StateType::Escrow,
            status.as_str(),
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
                op::listing_create().to_owned(),
                op::listing_cancel().to_owned(),
                op::listing_buy().to_owned(),
                op::listing_expire().to_owned(),
            ]
        );
        assert_eq!(
            EscrowRules.allowed_operations(),
            &[
                op::escrow_lock().to_owned(),
                op::escrow_release().to_owned(),
                op::escrow_refund().to_owned(),
            ]
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
        let listed = listing(listing_status::listed());
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
        let sold = listing(listing_status::sold());
        assert!(matches!(
            rules.check(&op("listing.cancel"), Some(&sold), &BTreeMap::new()),
            Err(ProfileError::InvalidTransition { from, .. }) if from == listing_status::sold().as_str()
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
        let locked = escrow(escrow_status::locked());
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
        let released = escrow(escrow_status::released());
        assert!(matches!(
            rules.check(&op("escrow.refund"), Some(&released), &BTreeMap::new()),
            Err(ProfileError::InvalidTransition { from, .. }) if from == escrow_status::released().as_str()
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
