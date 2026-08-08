//! Consumable stack profile.
//!
//! Stacked quantities that are consumed by use (protocol §20.4). A stack
//! carries a `subject`, a `quantity`, and a `unit` in its projected payload.
//! Quantities are canonical non-negative integer strings; the protocol bans
//! floating-point economic state (§10.3).

use std::collections::BTreeMap;

use statechronicle_core::amount::Amount;
use statechronicle_domain::intent::Operation;
use statechronicle_domain::state::StateProjection;
use statechronicle_domain::state_type::StateType;

use crate::error::ProfileError;
use crate::registry::{
    ProfileRules, input_amount, input_str, parse_amount_str, require_current, require_unborn,
};

/// Operations accepted by the consumable stack profile.
const OPERATIONS: &[&str] = &[
    "stack.create",
    "stack.credit",
    "stack.debit",
    "stack.consume",
    "stack.transfer",
    "stack.reserve",
    "stack.release",
    "stack.expire",
    "stack.adjust",
];

/// Rule set for [`StateType::ConsumableStack`] (protocol §20.4).
///
/// A stack has no status: every operation except `stack.create` requires an
/// existing resource, and quantity rules are applied per operation:
///
/// * `stack.create`: unborn resource with `subject`, non-negative `quantity`,
///   and `unit` inputs.
/// * `stack.credit`: existing resource; `quantity` is a non-negative integer.
/// * `stack.debit` / `stack.consume`: existing resource; `quantity` must be
///   strictly positive and no greater than the current quantity.
/// * `stack.transfer`: existing resource; `to_subject` plus a strictly
///   positive `quantity` no greater than the current quantity.
/// * `stack.reserve`: existing resource; strictly positive `quantity` no
///   greater than the current quantity.
/// * `stack.release`: existing resource; strictly positive `quantity`.
/// * `stack.adjust`: existing resource; new non-negative `quantity`.
/// * `stack.expire`: existing resource; terminal, no quantity required.
///
/// A stack never holds negative quantity, and amounts that would exceed the
/// current quantity are rejected with
/// [`ProfileError::InsufficientQuantity`].
#[derive(Debug, Clone, Copy)]
pub struct ConsumableStackRules;

impl ProfileRules for ConsumableStackRules {
    fn state_type(&self) -> StateType {
        StateType::ConsumableStack
    }

    fn profile_id(&self) -> &'static str {
        "consumable_stack"
    }

    fn allowed_operations(&self) -> &'static [&'static str] {
        OPERATIONS
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
            "stack.create" => check_create(current, inputs),
            "stack.credit" => check_credit(current, inputs),
            "stack.debit" => check_debit(current, inputs),
            "stack.consume" => check_consume(current, inputs),
            "stack.transfer" => check_transfer(current, inputs),
            "stack.reserve" => check_reserve(current, inputs),
            "stack.release" => check_release(current, inputs),
            "stack.expire" => check_expire(current),
            "stack.adjust" => check_adjust(current, inputs),
            _ => Err(ProfileError::UnknownOperation(String::from(
                operation.as_str(),
            ))),
        }
    }
}

/// Validates `stack.create`.
///
/// # Errors
///
/// Returns [`ProfileError::InvalidTransition`] when a stack already exists,
/// and [`ProfileError::InvalidInput`] / [`ProfileError::FloatForbidden`] for
/// missing or malformed `subject`, `unit`, or `quantity` inputs.
fn check_create(
    current: Option<&StateProjection>,
    inputs: &BTreeMap<String, serde_json::Value>,
) -> Result<(), ProfileError> {
    require_unborn(current, "stack.create")?;
    input_str(inputs, "subject")?;
    input_str(inputs, "unit")?;
    input_amount(inputs, "quantity")?;
    Ok(())
}

/// Validates `stack.credit`.
///
/// # Errors
///
/// Returns [`ProfileError::InvalidTransition`] when the stack does not exist,
/// and [`ProfileError::InvalidInput`] / [`ProfileError::FloatForbidden`] when
/// the `quantity` input is not a canonical non-negative integer string.
fn check_credit(
    current: Option<&StateProjection>,
    inputs: &BTreeMap<String, serde_json::Value>,
) -> Result<(), ProfileError> {
    require_current(current, "stack.credit")?;
    input_amount(inputs, "quantity")?;
    Ok(())
}

/// Validates `stack.debit`.
///
/// # Errors
///
/// Returns [`ProfileError::InvalidTransition`] when the stack does not exist,
/// [`ProfileError::NonPositiveAmount`] when the amount is zero,
/// [`ProfileError::InsufficientQuantity`] when it exceeds the current
/// quantity, and [`ProfileError::InvalidInput`] /
/// [`ProfileError::FloatForbidden`] for malformed inputs.
fn check_debit(
    current: Option<&StateProjection>,
    inputs: &BTreeMap<String, serde_json::Value>,
) -> Result<(), ProfileError> {
    let current = require_current(current, "stack.debit")?;
    let available = current_quantity(current)?;
    let requested = input_amount(inputs, "quantity")?;
    require_positive_available(requested, available, "stack.debit")
}

/// Validates `stack.consume`, which behaves exactly like `stack.debit`.
///
/// # Errors
///
/// Same error set as [`check_debit`].
fn check_consume(
    current: Option<&StateProjection>,
    inputs: &BTreeMap<String, serde_json::Value>,
) -> Result<(), ProfileError> {
    let current = require_current(current, "stack.consume")?;
    let available = current_quantity(current)?;
    let requested = input_amount(inputs, "quantity")?;
    require_positive_available(requested, available, "stack.consume")
}

/// Validates `stack.transfer`.
///
/// # Errors
///
/// Returns [`ProfileError::InvalidTransition`] when the stack does not exist,
/// [`ProfileError::InvalidInput`] when `to_subject` is missing/malformed,
/// [`ProfileError::NonPositiveAmount`] when the amount is zero,
/// [`ProfileError::InsufficientQuantity`] when it exceeds the current
/// quantity, and [`ProfileError::InvalidInput`] /
/// [`ProfileError::FloatForbidden`] for a malformed `quantity`.
fn check_transfer(
    current: Option<&StateProjection>,
    inputs: &BTreeMap<String, serde_json::Value>,
) -> Result<(), ProfileError> {
    let current = require_current(current, "stack.transfer")?;
    input_str(inputs, "to_subject")?;
    let available = current_quantity(current)?;
    let requested = input_amount(inputs, "quantity")?;
    require_positive_available(requested, available, "stack.transfer")
}

/// Validates `stack.reserve`.
///
/// # Errors
///
/// Returns [`ProfileError::InvalidTransition`] when the stack does not exist,
/// [`ProfileError::NonPositiveAmount`] when the amount is zero,
/// [`ProfileError::InsufficientQuantity`] when it exceeds the current
/// quantity, and [`ProfileError::InvalidInput`] /
/// [`ProfileError::FloatForbidden`] for malformed inputs.
fn check_reserve(
    current: Option<&StateProjection>,
    inputs: &BTreeMap<String, serde_json::Value>,
) -> Result<(), ProfileError> {
    let current = require_current(current, "stack.reserve")?;
    let available = current_quantity(current)?;
    let requested = input_amount(inputs, "quantity")?;
    require_positive_available(requested, available, "stack.reserve")
}

/// Validates `stack.release`.
///
/// # Errors
///
/// Returns [`ProfileError::InvalidTransition`] when the stack does not exist,
/// [`ProfileError::NonPositiveAmount`] when the amount is zero, and
/// [`ProfileError::InvalidInput`] / [`ProfileError::FloatForbidden`] for a
/// malformed `quantity`.
fn check_release(
    current: Option<&StateProjection>,
    inputs: &BTreeMap<String, serde_json::Value>,
) -> Result<(), ProfileError> {
    require_current(current, "stack.release")?;
    let requested = input_amount(inputs, "quantity")?;
    if requested.is_zero() {
        return Err(ProfileError::NonPositiveAmount);
    }
    Ok(())
}

/// Validates `stack.expire`, the terminal operation for a stack.
///
/// # Errors
///
/// Returns [`ProfileError::InvalidTransition`] when the stack does not exist.
fn check_expire(current: Option<&StateProjection>) -> Result<(), ProfileError> {
    require_current(current, "stack.expire")?;
    Ok(())
}

/// Validates `stack.adjust`, setting a new non-negative quantity.
///
/// # Errors
///
/// Returns [`ProfileError::InvalidTransition`] when the stack does not exist,
/// and [`ProfileError::InvalidInput`] / [`ProfileError::FloatForbidden`] when
/// the new `quantity` is not a canonical non-negative integer string.
fn check_adjust(
    current: Option<&StateProjection>,
    inputs: &BTreeMap<String, serde_json::Value>,
) -> Result<(), ProfileError> {
    require_current(current, "stack.adjust")?;
    input_amount(inputs, "quantity")?;
    Ok(())
}

/// Reads the current `quantity` from a stack's projected payload.
///
/// # Errors
///
/// Returns [`ProfileError::InvalidInput`] when the payload has no `quantity`
/// field or it is not a canonical non-negative integer string, and
/// [`ProfileError::FloatForbidden`] for float-formatted values.
fn current_quantity(current: &StateProjection) -> Result<Amount, ProfileError> {
    let value = current.state.get("quantity").ok_or_else(|| {
        ProfileError::InvalidInput(String::from("state payload has no `quantity`"))
    })?;
    parse_amount_str(value, "quantity")
}

/// Enforces `0 < requested <= available`.
///
/// # Errors
///
/// Returns [`ProfileError::NonPositiveAmount`] when `requested` is zero and
/// [`ProfileError::InsufficientQuantity`] when `requested` exceeds
/// `available`.
fn require_positive_available(
    requested: Amount,
    available: Amount,
    _operation: &str,
) -> Result<(), ProfileError> {
    if requested.is_zero() {
        return Err(ProfileError::NonPositiveAmount);
    }
    if requested > available {
        return Err(ProfileError::InsufficientQuantity {
            available,
            requested,
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

    fn stack(quantity: &str) -> StateProjection {
        StateProjection {
            tenant_id: TenantId(String::from("tenant.test")),
            resource_id: ResourceId(String::from("stack:arrows")),
            state_type: StateType::ConsumableStack,
            version: 1,
            last_event_id: EventId::new(String::from("evt_01JZ8X2XRE5ZYW5V9R7VDQBSH4")).unwrap(),
            last_commit_id: CommitId::new(String::from("cmt_01JZ8X5HN3C4PXG5A9FGEWQF5W")).unwrap(),
            state_hash: ContentDigest::new([0u8; 32]),
            state: serde_json::json!({
                "subject": "account:example:player_123",
                "quantity": quantity,
                "unit": "arrows"
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
            ConsumableStackRules.allowed_operations(),
            &[
                "stack.create",
                "stack.credit",
                "stack.debit",
                "stack.consume",
                "stack.transfer",
                "stack.reserve",
                "stack.release",
                "stack.expire",
                "stack.adjust",
            ][..]
        );
    }

    #[test]
    fn create_requires_unborn_and_quantity_fields() {
        let rules = ConsumableStackRules;
        let ok = inputs(&[
            ("subject", serde_json::json!("alice")),
            ("quantity", serde_json::json!("10")),
            ("unit", serde_json::json!("arrows")),
        ]);
        assert!(rules.check(&op("stack.create"), None, &ok).is_ok());

        assert!(matches!(
            rules.check(&op("stack.create"), None, &BTreeMap::new()),
            Err(ProfileError::InvalidInput(_))
        ));
        let existing = stack("10");
        assert!(matches!(
            rules.check(&op("stack.create"), Some(&existing), &ok),
            Err(ProfileError::InvalidTransition { from, .. }) if from == "existing"
        ));
    }

    #[test]
    fn floats_are_forbidden_everywhere() {
        let rules = ConsumableStackRules;
        let existing = stack("10");
        let float_input = inputs(&[("quantity", serde_json::json!("1.5"))]);
        assert!(matches!(
            rules.check(&op("stack.credit"), Some(&existing), &float_input),
            Err(ProfileError::FloatForbidden)
        ));
        assert!(matches!(
            rules.check(&op("stack.debit"), Some(&existing), &float_input),
            Err(ProfileError::FloatForbidden)
        ));
        // A float quantity in the projected state also fails closed.
        let broken = stack("3.5");
        assert!(matches!(
            rules.check(&op("stack.debit"), Some(&broken), &float_input),
            Err(ProfileError::FloatForbidden)
        ));
        // JSON numbers, not strings, are invalid input rather than floats.
        let number_input = inputs(&[("quantity", serde_json::json!(5))]);
        assert!(matches!(
            rules.check(&op("stack.credit"), Some(&existing), &number_input),
            Err(ProfileError::InvalidInput(_))
        ));
    }

    #[test]
    fn debit_and_consume_enforce_positive_and_available() {
        let rules = ConsumableStackRules;
        let existing = stack("10");

        assert!(
            rules
                .check(
                    &op("stack.debit"),
                    Some(&existing),
                    &inputs(&[("quantity", serde_json::json!("3"))])
                )
                .is_ok()
        );
        assert!(matches!(
            rules.check(
                &op("stack.debit"),
                Some(&existing),
                &inputs(&[("quantity", serde_json::json!("0"))])
            ),
            Err(ProfileError::NonPositiveAmount)
        ));
        assert!(matches!(
            rules.check(
                &op("stack.debit"),
                Some(&existing),
                &inputs(&[("quantity", serde_json::json!("11"))])
            ),
            Err(ProfileError::InsufficientQuantity { available, requested })
            if available == Amount::from_u64(10) && requested == Amount::from_u64(11)
        ));
        assert!(matches!(
            rules.check(
                &op("stack.consume"),
                Some(&existing),
                &inputs(&[("quantity", serde_json::json!("0"))])
            ),
            Err(ProfileError::NonPositiveAmount)
        ));
        // Consuming exactly the available quantity is allowed.
        assert!(
            rules
                .check(
                    &op("stack.consume"),
                    Some(&existing),
                    &inputs(&[("quantity", serde_json::json!("10"))])
                )
                .is_ok()
        );
    }

    #[test]
    fn transfer_requires_to_subject_and_available_quantity() {
        let rules = ConsumableStackRules;
        let existing = stack("10");
        assert!(
            rules
                .check(
                    &op("stack.transfer"),
                    Some(&existing),
                    &inputs(&[
                        ("to_subject", serde_json::json!("bob")),
                        ("quantity", serde_json::json!("4")),
                    ])
                )
                .is_ok()
        );
        assert!(matches!(
            rules.check(
                &op("stack.transfer"),
                Some(&existing),
                &inputs(&[("quantity", serde_json::json!("4"))])
            ),
            Err(ProfileError::InvalidInput(_))
        ));
        assert!(matches!(
            rules.check(
                &op("stack.transfer"),
                Some(&existing),
                &inputs(&[
                    ("to_subject", serde_json::json!("bob")),
                    ("quantity", serde_json::json!("11")),
                ])
            ),
            Err(ProfileError::InsufficientQuantity { .. })
        ));
    }

    #[test]
    fn reserve_release_adjust_expire() {
        let rules = ConsumableStackRules;
        let existing = stack("10");

        assert!(
            rules
                .check(
                    &op("stack.reserve"),
                    Some(&existing),
                    &inputs(&[("quantity", serde_json::json!("2"))])
                )
                .is_ok()
        );
        assert!(matches!(
            rules.check(
                &op("stack.reserve"),
                Some(&existing),
                &inputs(&[("quantity", serde_json::json!("11"))])
            ),
            Err(ProfileError::InsufficientQuantity { .. })
        ));

        assert!(
            rules
                .check(
                    &op("stack.release"),
                    Some(&existing),
                    &inputs(&[("quantity", serde_json::json!("1"))])
                )
                .is_ok()
        );
        assert!(matches!(
            rules.check(
                &op("stack.release"),
                Some(&existing),
                &inputs(&[("quantity", serde_json::json!("0"))])
            ),
            Err(ProfileError::NonPositiveAmount)
        ));

        assert!(
            rules
                .check(
                    &op("stack.adjust"),
                    Some(&existing),
                    &inputs(&[("quantity", serde_json::json!("0"))])
                )
                .is_ok()
        );
        assert!(
            rules
                .check(&op("stack.expire"), Some(&existing), &BTreeMap::new())
                .is_ok()
        );

        // None of these work on an unborn stack.
        for name in ["stack.credit", "stack.debit", "stack.expire"] {
            assert!(matches!(
                rules.check(&op(name), None, &BTreeMap::new()),
                Err(ProfileError::InvalidTransition { from, .. }) if from == "unborn"
            ));
        }
    }

    #[test]
    fn unknown_operations_are_rejected() {
        let rules = ConsumableStackRules;
        assert!(matches!(
            rules.check(&op("stack.teleport"), None, &BTreeMap::new()),
            Err(ProfileError::UnknownOperation(name)) if name == "stack.teleport"
        ));
    }
}
