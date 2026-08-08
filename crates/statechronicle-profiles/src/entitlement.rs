//! Entitlement profile.
//!
//! Grantable rights scoped to a subject (protocol §20.6). An entitlement
//! carries a `subject`, a `status`, and a `transferable` flag in its projected
//! payload and moves through an explicit status table.

use std::collections::BTreeMap;

use statechronicle_domain::intent::Operation;
use statechronicle_domain::state::StateProjection;
use statechronicle_domain::state_type::StateType;
use statechronicle_domain::status::Status;

use crate::error::ProfileError;
use crate::registry::{ProfileRules, input_str, require_from, require_unborn};

/// Typed wire status names for an entitlement.
pub mod status {
    use statechronicle_domain::status::Status;

    status_set! {
        /// The entitlement has been granted but is not yet active.
        granted => "granted";
        /// The entitlement is in force.
        active => "active";
        /// The entitlement is temporarily suspended.
        suspended => "suspended";
        /// The entitlement has expired (terminal).
        expired => "expired";
        /// The entitlement was revoked (terminal).
        revoked => "revoked";
    }
}

/// Typed operation constants accepted by the entitlement profile.
pub mod op {
    use statechronicle_domain::intent::Operation;

    op_set! {
        /// Grants a new entitlement.
        entitlement_grant => "entitlement.grant";
        /// Activates a granted entitlement.
        entitlement_activate => "entitlement.activate";
        /// Suspends an active entitlement.
        entitlement_suspend => "entitlement.suspend";
        /// Restores a suspended entitlement.
        entitlement_restore => "entitlement.restore";
        /// Expires an entitlement (terminal).
        entitlement_expire => "entitlement.expire";
        /// Revokes an entitlement (terminal).
        entitlement_revoke => "entitlement.revoke";
        /// Transfers a transferable entitlement.
        entitlement_transfer => "entitlement.transfer";
    }
}

/// Rule set for [`StateType::Entitlement`] (protocol §20.6).
///
/// # Transition table
///
/// | operation | from | to |
/// |---|---|---|
/// | `entitlement.grant` | *(no prior state)* | `granted` |
/// | `entitlement.activate` | `granted` | `active` |
/// | `entitlement.suspend` | `active` | `suspended` |
/// | `entitlement.restore` | `suspended` | `active` |
/// | `entitlement.expire` | `granted`/`active`/`suspended` | `expired` |
/// | `entitlement.revoke` | `granted`/`active` | `revoked` |
/// | `entitlement.transfer` | `granted`/`active` | *(unchanged)* |
///
/// `expired` and `revoked` are terminal. `entitlement.transfer` is permitted
/// only when the entitlement's payload carries `"transferable": true`;
/// otherwise it fails with [`ProfileError::NotTransferable`].
#[derive(Debug, Clone, Copy)]
pub struct EntitlementRules;

impl ProfileRules for EntitlementRules {
    fn state_type(&self) -> StateType {
        StateType::Entitlement
    }

    fn profile_id(&self) -> &'static str {
        "entitlement"
    }

    fn allowed_operations(&self) -> &'static [Operation] {
        op::all()
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
        if operation == op::entitlement_grant() {
            check_grant(current, inputs)
        } else if operation == op::entitlement_activate() {
            single_from(
                current,
                "entitlement.activate",
                &[status::granted().to_owned()],
            )
        } else if operation == op::entitlement_suspend() {
            single_from(
                current,
                "entitlement.suspend",
                &[status::active().to_owned()],
            )
        } else if operation == op::entitlement_restore() {
            single_from(
                current,
                "entitlement.restore",
                &[status::suspended().to_owned()],
            )
        } else if operation == op::entitlement_expire() {
            single_from(
                current,
                "entitlement.expire",
                &[
                    status::granted().to_owned(),
                    status::active().to_owned(),
                    status::suspended().to_owned(),
                ],
            )
        } else if operation == op::entitlement_revoke() {
            single_from(
                current,
                "entitlement.revoke",
                &[status::granted().to_owned(), status::active().to_owned()],
            )
        } else if operation == op::entitlement_transfer() {
            check_transfer(current, inputs)
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

/// Validates `entitlement.grant`: the entitlement must not exist yet.
///
/// The `subject` input is required; the optional `transferable` input, when
/// present, must be a boolean.
///
/// # Errors
///
/// Returns [`ProfileError::InvalidTransition`] when an entitlement already
/// exists, and [`ProfileError::InvalidInput`] when `subject` is
/// missing/malformed or `transferable` is not a boolean.
fn check_grant(
    current: Option<&StateProjection>,
    inputs: &BTreeMap<String, serde_json::Value>,
) -> Result<(), ProfileError> {
    require_unborn(current, "entitlement.grant")?;
    input_str(inputs, "subject")?;
    if let Some(value) = inputs.get("transferable")
        && !value.is_boolean()
    {
        return Err(ProfileError::InvalidInput(String::from(
            "`transferable` must be a boolean",
        )));
    }
    Ok(())
}

/// Validates `entitlement.transfer`.
///
/// Transfer is allowed from `granted` or `active`, requires a `to_subject`
/// input, and is only permitted when the entitlement's `transferable` flag is
/// true.
///
/// # Errors
///
/// Returns [`ProfileError::InvalidTransition`] when the entitlement does not
/// exist or is not `granted`/`active`, [`ProfileError::NotTransferable`] when
/// the entitlement is not transferable, and
/// [`ProfileError::InvalidInput`] when `to_subject` is missing/malformed or
/// the `transferable` flag is not a boolean.
fn check_transfer(
    current: Option<&StateProjection>,
    inputs: &BTreeMap<String, serde_json::Value>,
) -> Result<(), ProfileError> {
    let current = require_from(
        current,
        "entitlement.transfer",
        &[status::granted().to_owned(), status::active().to_owned()],
    )?;
    if !is_transferable(current)? {
        return Err(ProfileError::NotTransferable);
    }
    input_str(inputs, "to_subject")?;
    Ok(())
}

/// Reads the `transferable` flag from an entitlement's state payload.
///
/// # Errors
///
/// Returns [`ProfileError::InvalidInput`] when the payload has no
/// `transferable` field or it is not a boolean.
fn is_transferable(current: &StateProjection) -> Result<bool, ProfileError> {
    let value = current.state.get("transferable").ok_or_else(|| {
        ProfileError::InvalidInput(String::from("state payload has no `transferable`"))
    })?;
    value
        .as_bool()
        .ok_or_else(|| ProfileError::InvalidInput(String::from("`transferable` must be a boolean")))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;
    use statechronicle_core::digest::ContentDigest;
    use statechronicle_domain::ids::{CommitId, EventId};
    use statechronicle_domain::resource::ResourceId;
    use statechronicle_domain::tenant::TenantId;

    fn entitlement(status: &Status, transferable: bool) -> StateProjection {
        StateProjection {
            tenant_id: TenantId(String::from("tenant.test")),
            resource_id: ResourceId(String::from("entitlement:membership")),
            state_type: StateType::Entitlement,
            version: 1,
            last_event_id: EventId::new(String::from("evt_01JZ8X2XRE5ZYW5V9R7VDQBSH4")).unwrap(),
            last_commit_id: CommitId::new(String::from("cmt_01JZ8X5HN3C4PXG5A9FGEWQF5W")).unwrap(),
            state_hash: ContentDigest::new([0u8; 32]),
            state: serde_json::json!({
                "subject": "account:example:player_123",
                "status": status.as_str(),
                "transferable": transferable
            }),
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
        assert_eq!(
            EntitlementRules.allowed_operations(),
            &[
                op::entitlement_grant().to_owned(),
                op::entitlement_activate().to_owned(),
                op::entitlement_suspend().to_owned(),
                op::entitlement_restore().to_owned(),
                op::entitlement_expire().to_owned(),
                op::entitlement_revoke().to_owned(),
                op::entitlement_transfer().to_owned(),
            ]
        );
    }

    #[test]
    fn grant_requires_unborn_and_valid_transferable() {
        let rules = EntitlementRules;
        assert!(
            rules
                .check(
                    &op("entitlement.grant"),
                    None,
                    &inputs(&[
                        ("subject", serde_json::json!("alice")),
                        ("transferable", serde_json::json!(true)),
                    ])
                )
                .is_ok()
        );
        assert!(matches!(
            rules.check(
                &op("entitlement.grant"),
                None,
                &inputs(&[("transferable", serde_json::json!("yes"))])
            ),
            Err(ProfileError::InvalidInput(_))
        ));
        let existing = entitlement(status::granted(), true);
        assert!(matches!(
            rules.check(&op("entitlement.grant"), Some(&existing), &BTreeMap::new()),
            Err(ProfileError::InvalidTransition { from, .. }) if from == "existing"
        ));
    }

    #[test]
    fn lifecycle_transitions() {
        let rules = EntitlementRules;
        let granted = entitlement(status::granted(), true);
        assert!(
            rules
                .check(
                    &op("entitlement.activate"),
                    Some(&granted),
                    &BTreeMap::new()
                )
                .is_ok()
        );

        let active = entitlement(status::active(), true);
        assert!(
            rules
                .check(&op("entitlement.suspend"), Some(&active), &BTreeMap::new())
                .is_ok()
        );

        let suspended = entitlement(status::suspended(), true);
        assert!(
            rules
                .check(
                    &op("entitlement.restore"),
                    Some(&suspended),
                    &BTreeMap::new()
                )
                .is_ok()
        );

        // expire from granted/active/suspended.
        for from in [status::granted(), status::active(), status::suspended()] {
            let projection = entitlement(from, true);
            assert!(
                rules
                    .check(
                        &op("entitlement.expire"),
                        Some(&projection),
                        &BTreeMap::new()
                    )
                    .is_ok()
            );
        }

        // revoke from granted/active only.
        for from in [status::granted(), status::active()] {
            let projection = entitlement(from, true);
            assert!(
                rules
                    .check(
                        &op("entitlement.revoke"),
                        Some(&projection),
                        &BTreeMap::new()
                    )
                    .is_ok()
            );
        }
        assert!(matches!(
            rules.check(&op("entitlement.revoke"), Some(&suspended), &BTreeMap::new()),
            Err(ProfileError::InvalidTransition { from, .. }) if from == status::suspended().as_str()
        ));
    }

    #[test]
    fn transfer_requires_transferable_flag() {
        let rules = EntitlementRules;
        let active = entitlement(status::active(), true);
        let not_transferable = entitlement(status::active(), false);

        assert!(
            rules
                .check(
                    &op("entitlement.transfer"),
                    Some(&active),
                    &inputs(&[("to_subject", serde_json::json!("bob"))])
                )
                .is_ok()
        );
        assert!(matches!(
            rules.check(
                &op("entitlement.transfer"),
                Some(&not_transferable),
                &inputs(&[("to_subject", serde_json::json!("bob"))])
            ),
            Err(ProfileError::NotTransferable)
        ));

        // Transfer from suspended is disallowed even when transferable.
        let suspended = entitlement(status::suspended(), true);
        assert!(matches!(
            rules.check(
                &op("entitlement.transfer"),
                Some(&suspended),
                &inputs(&[("to_subject", serde_json::json!("bob"))])
            ),
            Err(ProfileError::InvalidTransition { from, .. }) if from == status::suspended().as_str()
        ));
    }

    #[test]
    fn terminal_states_accept_no_mutations() {
        let rules = EntitlementRules;
        for source in [status::expired(), status::revoked()] {
            let projection = entitlement(source, true);
            assert!(matches!(
                rules.check(&op("entitlement.activate"), Some(&projection), &BTreeMap::new()),
                Err(ProfileError::InvalidTransition { from, .. }) if from == source.as_str()
            ));
            assert!(matches!(
                rules.check(&op("entitlement.transfer"), Some(&projection), &BTreeMap::new()),
                Err(ProfileError::InvalidTransition { from, .. }) if from == source.as_str()
            ));
        }
    }

    #[test]
    fn unknown_operations_are_rejected() {
        let rules = EntitlementRules;
        assert!(matches!(
            rules.check(&op("entitlement.teleport"), None, &BTreeMap::new()),
            Err(ProfileError::UnknownOperation(name)) if name == "entitlement.teleport"
        ));
    }
}
