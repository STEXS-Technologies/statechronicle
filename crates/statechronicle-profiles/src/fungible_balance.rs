//! Fungible balance profile.
//!
//! Divisible balances supporting credit, debit, and transfer (protocol §20.5).
//! A balance carries a `subject`, a `balance`, and a `unit` in its projected
//! payload. All amounts are canonical non-negative integer strings. The
//! protocol bans floating-point economic state (§10.3), so any float anywhere
//! is rejected fail-closed.

use std::collections::BTreeMap;

use statechronicle_core::amount::Amount;
use statechronicle_domain::intent::Operation;
use statechronicle_domain::state::StateProjection;
use statechronicle_domain::state_type::StateType;

use crate::error::ProfileError;
use crate::registry::{
    ProfileRules, input_amount, input_str, parse_amount_str, require_current, require_unborn,
};

/// Operations accepted by the fungible balance profile.
const OPERATIONS: &[&str] = &[
    "balance.create",
    "balance.mint",
    "balance.credit",
    "balance.debit",
    "balance.transfer",
    "balance.reserve",
    "balance.release",
    "balance.spend",
    "balance.burn",
    "balance.convert",
];

/// Rule set for [`StateType::FungibleBalance`] (protocol §20.5).
///
/// A balance has no status: every operation except `balance.create` requires
/// an existing resource. Amount rules:
///
/// * `balance.create`: unborn resource with `subject`, non-negative `balance`,
///   and `unit` inputs.
/// * `balance.mint` / `balance.burn`: existing resource; strictly positive
///   `amount`, plus a non-empty `authorized_by` input (central-bank-style
///   authorization).
/// * `balance.credit` / `balance.release`: existing resource; strictly
///   positive `amount`.
/// * `balance.debit` / `balance.spend`: existing resource; strictly positive
///   `amount` no greater than the current balance.
/// * `balance.transfer`: existing resource; `to_subject` plus a strictly
///   positive `amount` no greater than the current balance.
/// * `balance.reserve`: existing resource; strictly positive `amount` no
///   greater than the current balance.
/// * `balance.convert`: existing resource; `to_unit` plus a strictly positive
///   `amount` no greater than the current balance.
///
/// Debits and spends beyond the current balance are rejected with
/// [`ProfileError::InsufficientQuantity`]; zero amounts with
/// [`ProfileError::NonPositiveAmount`].
#[derive(Debug, Clone, Copy)]
pub struct FungibleBalanceRules;

impl ProfileRules for FungibleBalanceRules {
    fn state_type(&self) -> StateType {
        StateType::FungibleBalance
    }

    fn profile_id(&self) -> &'static str {
        "fungible_balance"
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
            "balance.create" => check_create(current, inputs),
            "balance.mint" => check_mint(current, inputs),
            "balance.credit" => check_credit(current, inputs),
            "balance.debit" => check_debit(current, inputs),
            "balance.transfer" => check_transfer(current, inputs),
            "balance.reserve" => check_reserve(current, inputs),
            "balance.release" => check_release(current, inputs),
            "balance.spend" => check_spend(current, inputs),
            "balance.burn" => check_burn(current, inputs),
            "balance.convert" => check_convert(current, inputs),
            _ => Err(ProfileError::UnknownOperation(String::from(
                operation.as_str(),
            ))),
        }
    }
}

/// Validates `balance.create`.
///
/// # Errors
///
/// Returns [`ProfileError::InvalidTransition`] when a balance already exists,
/// and [`ProfileError::InvalidInput`] / [`ProfileError::FloatForbidden`] for
/// missing or malformed `subject`, `unit`, or `balance` inputs.
fn check_create(
    current: Option<&StateProjection>,
    inputs: &BTreeMap<String, serde_json::Value>,
) -> Result<(), ProfileError> {
    require_unborn(current, "balance.create")?;
    input_str(inputs, "subject")?;
    input_str(inputs, "unit")?;
    input_amount(inputs, "balance")?;
    Ok(())
}

/// Validates `balance.mint`.
///
/// # Errors
///
/// Returns [`ProfileError::InvalidTransition`] when the balance does not
/// exist, [`ProfileError::InvalidInput`] when `authorized_by` is
/// missing/empty or the `amount` is malformed, and
/// [`ProfileError::NonPositiveAmount`] when the amount is zero.
fn check_mint(
    current: Option<&StateProjection>,
    inputs: &BTreeMap<String, serde_json::Value>,
) -> Result<(), ProfileError> {
    let current = require_current(current, "balance.mint")?;
    input_str(inputs, "authorized_by")?;
    let amount = input_amount(inputs, "amount")?;
    let _balance = current_balance(current)?;
    require_positive(amount, "balance.mint")
}

/// Validates `balance.credit`.
///
/// # Errors
///
/// Returns [`ProfileError::InvalidTransition`] when the balance does not
/// exist, [`ProfileError::NonPositiveAmount`] when the amount is zero, and
/// [`ProfileError::InvalidInput`] / [`ProfileError::FloatForbidden`] for a
/// malformed `amount`.
fn check_credit(
    current: Option<&StateProjection>,
    inputs: &BTreeMap<String, serde_json::Value>,
) -> Result<(), ProfileError> {
    let current = require_current(current, "balance.credit")?;
    let amount = input_amount(inputs, "amount")?;
    let _balance = current_balance(current)?;
    require_positive(amount, "balance.credit")
}

/// Validates `balance.debit`.
///
/// # Errors
///
/// Returns [`ProfileError::InvalidTransition`] when the balance does not
/// exist, [`ProfileError::NonPositiveAmount`] when the amount is zero,
/// [`ProfileError::InsufficientQuantity`] when it exceeds the current balance,
/// and [`ProfileError::InvalidInput`] / [`ProfileError::FloatForbidden`] for
/// malformed inputs.
fn check_debit(
    current: Option<&StateProjection>,
    inputs: &BTreeMap<String, serde_json::Value>,
) -> Result<(), ProfileError> {
    let current = require_current(current, "balance.debit")?;
    let balance = current_balance(current)?;
    let amount = input_amount(inputs, "amount")?;
    require_positive_available(amount, balance, "balance.debit")
}

/// Validates `balance.transfer`.
///
/// # Errors
///
/// Returns [`ProfileError::InvalidTransition`] when the balance does not
/// exist, [`ProfileError::InvalidInput`] when `to_subject` is missing/malformed,
/// [`ProfileError::NonPositiveAmount`] when the amount is zero,
/// [`ProfileError::InsufficientQuantity`] when it exceeds the current balance,
/// and [`ProfileError::InvalidInput`] / [`ProfileError::FloatForbidden`] for a
/// malformed `amount`.
fn check_transfer(
    current: Option<&StateProjection>,
    inputs: &BTreeMap<String, serde_json::Value>,
) -> Result<(), ProfileError> {
    let current = require_current(current, "balance.transfer")?;
    input_str(inputs, "to_subject")?;
    let balance = current_balance(current)?;
    let amount = input_amount(inputs, "amount")?;
    require_positive_available(amount, balance, "balance.transfer")
}

/// Validates `balance.reserve`.
///
/// # Errors
///
/// Returns [`ProfileError::InvalidTransition`] when the balance does not
/// exist, [`ProfileError::NonPositiveAmount`] when the amount is zero,
/// [`ProfileError::InsufficientQuantity`] when it exceeds the current balance,
/// and [`ProfileError::InvalidInput`] / [`ProfileError::FloatForbidden`] for
/// malformed inputs.
fn check_reserve(
    current: Option<&StateProjection>,
    inputs: &BTreeMap<String, serde_json::Value>,
) -> Result<(), ProfileError> {
    let current = require_current(current, "balance.reserve")?;
    let balance = current_balance(current)?;
    let amount = input_amount(inputs, "amount")?;
    require_positive_available(amount, balance, "balance.reserve")
}

/// Validates `balance.release`.
///
/// # Errors
///
/// Returns [`ProfileError::InvalidTransition`] when the balance does not
/// exist, [`ProfileError::NonPositiveAmount`] when the amount is zero, and
/// [`ProfileError::InvalidInput`] / [`ProfileError::FloatForbidden`] for a
/// malformed `amount`.
fn check_release(
    current: Option<&StateProjection>,
    inputs: &BTreeMap<String, serde_json::Value>,
) -> Result<(), ProfileError> {
    let current = require_current(current, "balance.release")?;
    let amount = input_amount(inputs, "amount")?;
    let _balance = current_balance(current)?;
    require_positive(amount, "balance.release")
}

/// Validates `balance.spend`, which behaves like `balance.debit`.
///
/// # Errors
///
/// Same error set as [`check_debit`].
fn check_spend(
    current: Option<&StateProjection>,
    inputs: &BTreeMap<String, serde_json::Value>,
) -> Result<(), ProfileError> {
    let current = require_current(current, "balance.spend")?;
    let balance = current_balance(current)?;
    let amount = input_amount(inputs, "amount")?;
    require_positive_available(amount, balance, "balance.spend")
}

/// Validates `balance.burn`.
///
/// # Errors
///
/// Returns [`ProfileError::InvalidTransition`] when the balance does not
/// exist, [`ProfileError::InvalidInput`] when `authorized_by` is
/// missing/empty, [`ProfileError::NonPositiveAmount`] when the amount is zero,
/// [`ProfileError::InsufficientQuantity`] when it exceeds the current balance,
/// and [`ProfileError::InvalidInput`] / [`ProfileError::FloatForbidden`] for a
/// malformed `amount`.
fn check_burn(
    current: Option<&StateProjection>,
    inputs: &BTreeMap<String, serde_json::Value>,
) -> Result<(), ProfileError> {
    let current = require_current(current, "balance.burn")?;
    input_str(inputs, "authorized_by")?;
    let balance = current_balance(current)?;
    let amount = input_amount(inputs, "amount")?;
    require_positive_available(amount, balance, "balance.burn")
}

/// Validates `balance.convert`, converting `amount` into a new `unit`.
///
/// # Errors
///
/// Returns [`ProfileError::InvalidTransition`] when the balance does not
/// exist, [`ProfileError::InvalidInput`] when `to_unit` is missing/malformed,
/// [`ProfileError::NonPositiveAmount`] when the amount is zero,
/// [`ProfileError::InsufficientQuantity`] when it exceeds the current balance,
/// and [`ProfileError::InvalidInput`] / [`ProfileError::FloatForbidden`] for a
/// malformed `amount`.
fn check_convert(
    current: Option<&StateProjection>,
    inputs: &BTreeMap<String, serde_json::Value>,
) -> Result<(), ProfileError> {
    let current = require_current(current, "balance.convert")?;
    input_str(inputs, "to_unit")?;
    let balance = current_balance(current)?;
    let amount = input_amount(inputs, "amount")?;
    require_positive_available(amount, balance, "balance.convert")
}

/// Reads the current `balance` from a projection's state payload.
///
/// # Errors
///
/// Returns [`ProfileError::InvalidInput`] when the payload has no `balance`
/// field or it is not a canonical non-negative integer string, and
/// [`ProfileError::FloatForbidden`] for float-formatted values.
fn current_balance(current: &StateProjection) -> Result<Amount, ProfileError> {
    let value = current.state.get("balance").ok_or_else(|| {
        ProfileError::InvalidInput(String::from("state payload has no `balance`"))
    })?;
    parse_amount_str(value, "balance")
}

/// Enforces a strictly positive amount.
///
/// # Errors
///
/// Returns [`ProfileError::NonPositiveAmount`] when `amount` is zero.
const fn require_positive(amount: Amount, _operation: &str) -> Result<(), ProfileError> {
    if amount.is_zero() {
        return Err(ProfileError::NonPositiveAmount);
    }
    Ok(())
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

    fn balance(balance: &str) -> StateProjection {
        StateProjection {
            tenant_id: TenantId(String::from("tenant.test")),
            resource_id: ResourceId(String::from("balance:gold")),
            state_type: StateType::FungibleBalance,
            version: 1,
            last_event_id: EventId::new(String::from("evt_01JZ8X2XRE5ZYW5V9R7VDQBSH4")).unwrap(),
            last_commit_id: CommitId::new(String::from("cmt_01JZ8X5HN3C4PXG5A9FGEWQF5W")).unwrap(),
            state_hash: ContentDigest::new([0u8; 32]),
            state: serde_json::json!({
                "subject": "account:example:player_123",
                "balance": balance,
                "unit": "gold_minor"
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
            FungibleBalanceRules.allowed_operations(),
            &[
                "balance.create",
                "balance.mint",
                "balance.credit",
                "balance.debit",
                "balance.transfer",
                "balance.reserve",
                "balance.release",
                "balance.spend",
                "balance.burn",
                "balance.convert",
            ][..]
        );
    }

    #[test]
    fn create_requires_unborn_and_fields() {
        let rules = FungibleBalanceRules;
        let ok = inputs(&[
            ("subject", serde_json::json!("alice")),
            ("balance", serde_json::json!("1000")),
            ("unit", serde_json::json!("gold_minor")),
        ]);
        assert!(rules.check(&op("balance.create"), None, &ok).is_ok());
        let existing = balance("1000");
        assert!(matches!(
            rules.check(&op("balance.create"), Some(&existing), &ok),
            Err(ProfileError::InvalidTransition { from, .. }) if from == "existing"
        ));
    }

    #[test]
    fn mint_and_burn_require_authorized_by() {
        let rules = FungibleBalanceRules;
        let existing = balance("1000");
        assert!(
            rules
                .check(
                    &op("balance.mint"),
                    Some(&existing),
                    &inputs(&[
                        ("amount", serde_json::json!("50")),
                        ("authorized_by", serde_json::json!("treasury")),
                    ])
                )
                .is_ok()
        );
        assert!(matches!(
            rules.check(
                &op("balance.mint"),
                Some(&existing),
                &inputs(&[("amount", serde_json::json!("50"))])
            ),
            Err(ProfileError::InvalidInput(_))
        ));
        assert!(matches!(
            rules.check(
                &op("balance.burn"),
                Some(&existing),
                &inputs(&[("amount", serde_json::json!("50"))])
            ),
            Err(ProfileError::InvalidInput(_))
        ));
        assert!(matches!(
            rules.check(
                &op("balance.burn"),
                Some(&existing),
                &inputs(&[
                    ("amount", serde_json::json!("50")),
                    ("authorized_by", serde_json::json!("")),
                ])
            ),
            Err(ProfileError::InvalidInput(_))
        ));
    }

    #[test]
    fn debit_and_spend_enforce_positive_and_available() {
        let rules = FungibleBalanceRules;
        let existing = balance("1000");
        assert!(
            rules
                .check(
                    &op("balance.debit"),
                    Some(&existing),
                    &inputs(&[("amount", serde_json::json!("100"))])
                )
                .is_ok()
        );
        assert!(matches!(
            rules.check(
                &op("balance.debit"),
                Some(&existing),
                &inputs(&[("amount", serde_json::json!("0"))])
            ),
            Err(ProfileError::NonPositiveAmount)
        ));
        assert!(matches!(
            rules.check(
                &op("balance.spend"),
                Some(&existing),
                &inputs(&[("amount", serde_json::json!("1001"))])
            ),
            Err(ProfileError::InsufficientQuantity { available, requested })
            if available == Amount::from_u64(1000) && requested == Amount::from_u64(1001)
        ));
    }

    #[test]
    fn credit_transfer_reserve_release_convert() {
        let rules = FungibleBalanceRules;
        let existing = balance("1000");

        assert!(
            rules
                .check(
                    &op("balance.credit"),
                    Some(&existing),
                    &inputs(&[("amount", serde_json::json!("5"))])
                )
                .is_ok()
        );

        assert!(
            rules
                .check(
                    &op("balance.transfer"),
                    Some(&existing),
                    &inputs(&[
                        ("to_subject", serde_json::json!("bob")),
                        ("amount", serde_json::json!("10")),
                    ])
                )
                .is_ok()
        );
        assert!(matches!(
            rules.check(
                &op("balance.transfer"),
                Some(&existing),
                &inputs(&[
                    ("to_subject", serde_json::json!("bob")),
                    ("amount", serde_json::json!("0")),
                ])
            ),
            Err(ProfileError::NonPositiveAmount)
        ));

        assert!(
            rules
                .check(
                    &op("balance.reserve"),
                    Some(&existing),
                    &inputs(&[("amount", serde_json::json!("20"))])
                )
                .is_ok()
        );
        assert!(
            rules
                .check(
                    &op("balance.release"),
                    Some(&existing),
                    &inputs(&[("amount", serde_json::json!("20"))])
                )
                .is_ok()
        );

        assert!(
            rules
                .check(
                    &op("balance.convert"),
                    Some(&existing),
                    &inputs(&[
                        ("to_unit", serde_json::json!("gold_major")),
                        ("amount", serde_json::json!("100")),
                    ])
                )
                .is_ok()
        );
        assert!(matches!(
            rules.check(
                &op("balance.convert"),
                Some(&existing),
                &inputs(&[("amount", serde_json::json!("100"))])
            ),
            Err(ProfileError::InvalidInput(_))
        ));
    }

    #[test]
    fn no_floats_anywhere() {
        let rules = FungibleBalanceRules;
        let existing = balance("1000");
        assert!(matches!(
            rules.check(
                &op("balance.credit"),
                Some(&existing),
                &inputs(&[("amount", serde_json::json!("1.0"))])
            ),
            Err(ProfileError::FloatForbidden)
        ));
        let float_state = balance("10.5");
        assert!(matches!(
            rules.check(
                &op("balance.credit"),
                Some(&float_state),
                &inputs(&[("amount", serde_json::json!("1"))])
            ),
            Err(ProfileError::FloatForbidden)
        ));
    }

    #[test]
    fn unknown_operations_are_rejected() {
        let rules = FungibleBalanceRules;
        assert!(matches!(
            rules.check(&op("balance.teleport"), None, &BTreeMap::new()),
            Err(ProfileError::UnknownOperation(name)) if name == "balance.teleport"
        ));
    }
}
