//! Meter profile.
//!
//! Usage meters that accrue and are settled against entitlements (protocol
//! §20.7). A meter carries a `subject`, a `remaining` quantity, and a
//! `maximum` in its projected payload, all as canonical non-negative integer
//! strings.
//!
//! Refill is **deterministic**: `meter.refill` sets `remaining` to `maximum`.
//! `meter.set_maximum` sets a new non-negative `maximum` and clamps `remaining`
//! down to the new maximum. `meter.reset` sets `remaining` to zero.
//! `meter.expire` is terminal.

use std::collections::BTreeMap;

use statechronicle_core::amount::Amount;
use statechronicle_domain::intent::Operation;
use statechronicle_domain::state::StateProjection;
use statechronicle_domain::state_type::StateType;

use crate::error::ProfileError;
use crate::registry::{
    ProfileRules, input_amount, input_str, parse_amount_str, require_current, require_unborn,
};

/// Operations accepted by the meter profile.
const OPERATIONS: &[&str] = &[
    "meter.create",
    "meter.consume",
    "meter.refill",
    "meter.set_maximum",
    "meter.reset",
    "meter.expire",
];

/// Rule set for [`StateType::MeteredResource`] (protocol §20.7).
///
/// * `meter.create`: unborn resource with `subject`, non-negative `remaining`,
///   and non-negative `maximum` inputs; `remaining` may not exceed `maximum`.
/// * `meter.consume`: existing resource; strictly positive `amount` no
///   greater than `remaining`.
/// * `meter.refill`: existing resource; deterministically sets `remaining`
///   to `maximum` (the rule validates that both fields are present and valid).
/// * `meter.set_maximum`: existing resource; new non-negative `maximum`; the
///   post-state rule clamps `remaining` down to the new maximum.
/// * `meter.reset`: existing resource; sets `remaining` to zero.
/// * `meter.expire`: existing resource; terminal.
///
/// The invariant `remaining <= maximum` is enforced at creation and
/// preserved by every operation.
#[derive(Debug, Clone, Copy)]
pub struct MeterRules;

impl ProfileRules for MeterRules {
    fn state_type(&self) -> StateType {
        StateType::MeteredResource
    }

    fn profile_id(&self) -> &'static str {
        "meter"
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
            "meter.create" => check_create(current, inputs),
            "meter.consume" => check_consume(current, inputs),
            "meter.refill" => check_refill(current),
            "meter.set_maximum" => check_set_maximum(current, inputs),
            "meter.reset" => check_reset(current),
            "meter.expire" => check_expire(current),
            _ => Err(ProfileError::UnknownOperation(String::from(
                operation.as_str(),
            ))),
        }
    }
}

/// Validates `meter.create`.
///
/// # Errors
///
/// Returns [`ProfileError::InvalidTransition`] when a meter already exists,
/// [`ProfileError::InvalidInput`] / [`ProfileError::FloatForbidden`] for
/// missing or malformed `subject`, `remaining`, or `maximum` inputs, and
/// [`ProfileError::InvalidInput`] when `remaining` exceeds `maximum`.
fn check_create(
    current: Option<&StateProjection>,
    inputs: &BTreeMap<String, serde_json::Value>,
) -> Result<(), ProfileError> {
    require_unborn(current, "meter.create")?;
    input_str(inputs, "subject")?;
    let remaining = input_amount(inputs, "remaining")?;
    let maximum = input_amount(inputs, "maximum")?;
    if remaining > maximum {
        return Err(ProfileError::InvalidInput(String::from(
            "`remaining` must not exceed `maximum`",
        )));
    }
    Ok(())
}

/// Validates `meter.consume`.
///
/// # Errors
///
/// Returns [`ProfileError::InvalidTransition`] when the meter does not exist,
/// [`ProfileError::NonPositiveAmount`] when the amount is zero,
/// [`ProfileError::InsufficientQuantity`] when it exceeds `remaining`, and
/// [`ProfileError::InvalidInput`] / [`ProfileError::FloatForbidden`] for
/// malformed inputs.
fn check_consume(
    current: Option<&StateProjection>,
    inputs: &BTreeMap<String, serde_json::Value>,
) -> Result<(), ProfileError> {
    let current = require_current(current, "meter.consume")?;
    let remaining = current_remaining(current)?;
    let amount = input_amount(inputs, "amount")?;
    if amount.is_zero() {
        return Err(ProfileError::NonPositiveAmount);
    }
    if amount > remaining {
        return Err(ProfileError::InsufficientQuantity {
            available: remaining,
            requested: amount,
        });
    }
    Ok(())
}

/// Validates `meter.refill`.
///
/// Refill deterministically sets `remaining` to `maximum`; the rule requires
/// both fields to be present and valid in the projected payload.
///
/// # Errors
///
/// Returns [`ProfileError::InvalidTransition`] when the meter does not exist,
/// and [`ProfileError::InvalidInput`] / [`ProfileError::FloatForbidden`] when
/// `remaining` or `maximum` is missing or malformed.
fn check_refill(current: Option<&StateProjection>) -> Result<(), ProfileError> {
    let current = require_current(current, "meter.refill")?;
    let remaining = current_remaining(current)?;
    let _maximum = current_maximum(current)?;
    // Deterministic rule: refill sets remaining to maximum. The resulting
    // quantity is bounded by the invariant, so the pre-state only needs to be
    // valid and parseable.
    let _ = remaining;
    Ok(())
}

/// Validates `meter.set_maximum`.
///
/// The post-state rule clamps `remaining` down to the new `maximum`, so the
/// invariant `remaining <= maximum` is preserved by construction.
///
/// # Errors
///
/// Returns [`ProfileError::InvalidTransition`] when the meter does not exist,
/// and [`ProfileError::InvalidInput`] / [`ProfileError::FloatForbidden`] when
/// the new `maximum` is missing or malformed.
fn check_set_maximum(
    current: Option<&StateProjection>,
    inputs: &BTreeMap<String, serde_json::Value>,
) -> Result<(), ProfileError> {
    require_current(current, "meter.set_maximum")?;
    input_amount(inputs, "maximum")?;
    Ok(())
}

/// Validates `meter.reset`, which sets `remaining` to zero.
///
/// # Errors
///
/// Returns [`ProfileError::InvalidTransition`] when the meter does not exist.
fn check_reset(current: Option<&StateProjection>) -> Result<(), ProfileError> {
    require_current(current, "meter.reset")?;
    Ok(())
}

/// Validates `meter.expire`, the terminal operation for a meter.
///
/// # Errors
///
/// Returns [`ProfileError::InvalidTransition`] when the meter does not exist.
fn check_expire(current: Option<&StateProjection>) -> Result<(), ProfileError> {
    require_current(current, "meter.expire")?;
    Ok(())
}

/// Reads the current `remaining` from a meter's projected payload.
///
/// # Errors
///
/// Returns [`ProfileError::InvalidInput`] when the payload has no `remaining`
/// field or it is not a canonical non-negative integer string, and
/// [`ProfileError::FloatForbidden`] for float-formatted values.
fn current_remaining(current: &StateProjection) -> Result<Amount, ProfileError> {
    let value = current.state.get("remaining").ok_or_else(|| {
        ProfileError::InvalidInput(String::from("state payload has no `remaining`"))
    })?;
    parse_amount_str(value, "remaining")
}

/// Reads the current `maximum` from a meter's projected payload.
///
/// # Errors
///
/// Returns [`ProfileError::InvalidInput`] when the payload has no `maximum`
/// field or it is not a canonical non-negative integer string, and
/// [`ProfileError::FloatForbidden`] for float-formatted values.
fn current_maximum(current: &StateProjection) -> Result<Amount, ProfileError> {
    let value = current.state.get("maximum").ok_or_else(|| {
        ProfileError::InvalidInput(String::from("state payload has no `maximum`"))
    })?;
    parse_amount_str(value, "maximum")
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;
    use statechronicle_core::digest::ContentDigest;
    use statechronicle_domain::ids::{CommitId, EventId};
    use statechronicle_domain::resource::ResourceId;
    use statechronicle_domain::tenant::TenantId;

    fn meter(remaining: &str, maximum: &str) -> StateProjection {
        StateProjection {
            tenant_id: TenantId(String::from("tenant.test")),
            resource_id: ResourceId(String::from("meter:bandwidth")),
            state_type: StateType::MeteredResource,
            version: 1,
            last_event_id: EventId::new(String::from("evt_01JZ8X2XRE5ZYW5V9R7VDQBSH4")).unwrap(),
            last_commit_id: CommitId::new(String::from("cmt_01JZ8X5HN3C4PXG5A9FGEWQF5W")).unwrap(),
            state_hash: ContentDigest::new([0u8; 32]),
            state: serde_json::json!({
                "subject": "account:example:player_123",
                "remaining": remaining,
                "maximum": maximum
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
            MeterRules.allowed_operations(),
            &[
                "meter.create",
                "meter.consume",
                "meter.refill",
                "meter.set_maximum",
                "meter.reset",
                "meter.expire",
            ][..]
        );
    }

    #[test]
    fn create_requires_remaining_not_above_maximum() {
        let rules = MeterRules;
        let ok = inputs(&[
            ("subject", serde_json::json!("alice")),
            ("remaining", serde_json::json!("40")),
            ("maximum", serde_json::json!("100")),
        ]);
        assert!(rules.check(&op("meter.create"), None, &ok).is_ok());

        let over = inputs(&[
            ("subject", serde_json::json!("alice")),
            ("remaining", serde_json::json!("101")),
            ("maximum", serde_json::json!("100")),
        ]);
        assert!(matches!(
            rules.check(&op("meter.create"), None, &over),
            Err(ProfileError::InvalidInput(_))
        ));
    }

    #[test]
    fn consume_enforces_positive_and_remaining() {
        let rules = MeterRules;
        let existing = meter("40", "100");
        assert!(
            rules
                .check(
                    &op("meter.consume"),
                    Some(&existing),
                    &inputs(&[("amount", serde_json::json!("5"))])
                )
                .is_ok()
        );
        assert!(matches!(
            rules.check(
                &op("meter.consume"),
                Some(&existing),
                &inputs(&[("amount", serde_json::json!("0"))])
            ),
            Err(ProfileError::NonPositiveAmount)
        ));
        assert!(matches!(
            rules.check(
                &op("meter.consume"),
                Some(&existing),
                &inputs(&[("amount", serde_json::json!("41"))])
            ),
            Err(ProfileError::InsufficientQuantity { available, requested })
            if available == Amount::from_u64(40) && requested == Amount::from_u64(41)
        ));
    }

    #[test]
    fn refill_is_deterministic_remaining_equals_maximum() {
        // The documented post-state rule: refill sets remaining to maximum.
        // The pure checker validates both fields are present and valid.
        let rules = MeterRules;
        let existing = meter("10", "100");
        assert!(
            rules
                .check(&op("meter.refill"), Some(&existing), &BTreeMap::new())
                .is_ok()
        );

        let broken = meter("10", "10.5");
        assert!(matches!(
            rules.check(&op("meter.refill"), Some(&broken), &BTreeMap::new()),
            Err(ProfileError::FloatForbidden)
        ));
    }

    #[test]
    fn set_maximum_reset_expire() {
        let rules = MeterRules;
        let existing = meter("40", "100");
        assert!(
            rules
                .check(
                    &op("meter.set_maximum"),
                    Some(&existing),
                    &inputs(&[("maximum", serde_json::json!("200"))])
                )
                .is_ok()
        );
        assert!(
            rules
                .check(&op("meter.reset"), Some(&existing), &BTreeMap::new())
                .is_ok()
        );
        assert!(
            rules
                .check(&op("meter.expire"), Some(&existing), &BTreeMap::new())
                .is_ok()
        );

        // None of these work on an unborn meter.
        for name in [
            "meter.consume",
            "meter.refill",
            "meter.set_maximum",
            "meter.reset",
            "meter.expire",
        ] {
            assert!(matches!(
                rules.check(&op(name), None, &BTreeMap::new()),
                Err(ProfileError::InvalidTransition { from, .. }) if from == "unborn"
            ));
        }
    }

    #[test]
    fn unknown_operations_are_rejected() {
        let rules = MeterRules;
        assert!(matches!(
            rules.check(&op("meter.teleport"), None, &BTreeMap::new()),
            Err(ProfileError::UnknownOperation(name)) if name == "meter.teleport"
        ));
    }
}
