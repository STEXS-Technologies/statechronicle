//! Deterministic after-state rules.
//!
//! For each state type, computes the unique after-state for a validated
//! transition (protocol §18.1 step 10). [`apply`] is a pure, total function:
//! the same `(before, operation, inputs)` always produces the same after-state
//! payload, and any unknown operation, malformed input, integer overflow, or
//! underflow fails closed with [`ExecutorError::TransitionInvalid`] instead of
//! panicking.
//!
//! Version increments are the pipeline's job, not this module's: [`apply`]
//! returns only the new profile projection payload (the `state` JSON value),
//! never a full [`StateProjection`].
//!
//! Amounts are canonical non-negative integer strings (protocol §10.3 bans
//! floating-point economic state): every quantity, balance, and meter value is
//! parsed with checked fixed-point [`Amount`] arithmetic and never touches floats.

use std::collections::BTreeMap;

use serde_json::{Value, json};

use statechronicle_accumulator::key::StateKey;
use statechronicle_core::amount::Amount;
use statechronicle_domain::intent::Operation;
use statechronicle_domain::resource::ResourceId;
use statechronicle_domain::state::StateProjection;
use statechronicle_domain::state_type::StateType;
use statechronicle_domain::subject::SubjectId;
use statechronicle_domain::tenant::TenantId;
use statechronicle_profiles::consumable_stack::op as stack_op;
use statechronicle_profiles::entitlement::op as entitlement_op;
use statechronicle_profiles::entitlement::status as entitlement_status;
use statechronicle_profiles::fungible_balance::op as balance_op;
use statechronicle_profiles::marketplace::escrow_status;
use statechronicle_profiles::marketplace::listing_status;
use statechronicle_profiles::marketplace::op as marketplace_op;
use statechronicle_profiles::meter::op as meter_op;
use statechronicle_profiles::paid_unique_asset::op as paid_op;
use statechronicle_profiles::unique_asset::op as asset_op;
use statechronicle_profiles::unique_asset::status as asset_status;

use crate::error::ExecutorError;

/// Computes the deterministic after-state payload for a transition.
///
/// `before` is the resource's current projection, or `None` when the resource
/// does not exist yet (a create or mint). The returned JSON value is the new
/// profile projection payload (owner/status, balance/unit, quantity, ...), not
/// a full [`StateProjection`]. Version increments are applied by the pipeline.
///
/// The state type is taken from `before.state_type` when a projection exists,
/// otherwise inferred from the operation prefix (`asset.*`, `stack.*`,
/// `balance.*`, `entitlement.*`, `meter.*`, `listing.*`, `escrow.*`).
///
/// For transfer operations this is the **source debit** after-state only
/// (source debited, holder preserved). The matching destination credit is
/// computed by [`transfer_after_state`]. The pipeline emits both events so a
/// transfer is an atomic debit + credit pair (§20.5).
///
/// # Errors
///
/// Returns [`ExecutorError::TransitionInvalid`] for an unknown operation,
/// an unknown state type, a missing or malformed input, a float-formatted
/// amount, or checked integer overflow/underflow in after-state arithmetic.
pub fn apply(
    before: Option<&StateProjection>,
    operation: &Operation,
    inputs: &BTreeMap<String, Value>,
) -> Result<Value, ExecutorError> {
    let state_type = state_type_of(before, operation)?;
    match state_type {
        StateType::UniqueAsset => apply_unique_asset(before, operation, inputs),
        StateType::ConsumableStack => apply_stack(before, operation, inputs),
        StateType::FungibleBalance => apply_balance(before, operation, inputs),
        StateType::Entitlement => apply_entitlement(before, operation, inputs),
        StateType::MeteredResource => apply_meter(before, operation, inputs),
        StateType::Listing => apply_listing(before, operation, inputs),
        StateType::Escrow => apply_escrow(before, operation, inputs),
    }
}

/// Computes the **destination credit** after-state payload for a transfer
/// (protocol §20.5 "transfers are atomic debit + credit transactions").
///
/// `source` is the source projection being debited (its `unit`/denomination is
/// preserved on the destination) and `destination` is the destination's current
/// projection, or `None` when it does not exist yet (create-on-credit at
/// version 0). The returned JSON value is the credited destination payload:
/// `subject = to_subject`, `quantity`/`balance = existing + amount`, with the
/// source's `unit`.
///
/// Only `stack.transfer` (credits `quantity`) and `balance.transfer` (credits
/// `balance`) are supported. Version increments are the pipeline's job.
///
/// # Errors
///
/// Returns [`ExecutorError::TransitionInvalid`] when `operation` is not a
/// subject-held transfer, a required input (`to_subject`, the amount) is
/// missing or malformed, the source lacks a `unit`, or the credited amount
/// overflows the `u128` mantissa.
pub fn transfer_after_state(
    source: &StateProjection,
    destination: Option<&StateProjection>,
    operation: &Operation,
    inputs: &BTreeMap<String, Value>,
) -> Result<Value, ExecutorError> {
    if operation == stack_op::stack_transfer() {
        let to_subject = input_str(inputs, "to_subject")?;
        let unit = state_str(source, "unit")?;
        let amount = input_amount(inputs, "quantity")?;
        let existing = destination
            .map(|projection| state_amount(projection, "quantity"))
            .transpose()?
            .unwrap_or(Amount::ZERO);
        let next = existing
            .checked_add(amount)
            .ok_or_else(|| overflow("stack quantity"))?;
        Ok(json!({
            "subject": to_subject,
            "quantity": next.to_canonical_string(),
            "unit": unit,
        }))
    } else if operation == balance_op::balance_transfer() {
        let to_subject = input_str(inputs, "to_subject")?;
        let unit = state_str(source, "unit")?;
        let amount = input_amount(inputs, "amount")?;
        let existing = destination
            .map(|projection| state_amount(projection, "balance"))
            .transpose()?
            .unwrap_or(Amount::ZERO);
        let next = existing
            .checked_add(amount)
            .ok_or_else(|| overflow("balance"))?;
        Ok(json!({
            "subject": to_subject,
            "balance": next.to_canonical_string(),
            "unit": unit,
        }))
    } else {
        Err(ExecutorError::TransitionInvalid(format!(
            "destination credit not defined for operation `{}`",
            operation.as_str()
        )))
    }
}

/// Derives the accumulator state key for a resource (ADR-005 §2).
///
/// Subject-held types ([`StateType::ConsumableStack`],
/// [`StateType::FungibleBalance`], [`StateType::Entitlement`],
/// [`StateType::MeteredResource`]) key by `(tenant, resource, subject)` via
/// [`StateKey::for_subject_held`]; owner-based types ([`StateType::UniqueAsset`],
/// [`StateType::Listing`], [`StateType::Escrow`]) key by `(tenant, resource)`
/// via [`StateKey::for_resource`].
///
/// # Errors
///
/// Returns [`ExecutorError::TransitionInvalid`] when `subject` is `None` for a
/// subject-held state type.
pub fn state_key_for(
    state_type: StateType,
    tenant: &TenantId,
    subject: Option<&SubjectId>,
    resource: &ResourceId,
) -> Result<StateKey, ExecutorError> {
    match state_type {
        StateType::ConsumableStack
        | StateType::FungibleBalance
        | StateType::Entitlement
        | StateType::MeteredResource => {
            let subject = subject.ok_or_else(|| {
                ExecutorError::TransitionInvalid(String::from(
                    "subject required for subject-held state type",
                ))
            })?;
            Ok(StateKey::for_subject_held(
                &tenant.0,
                &resource.0,
                &subject.0,
            ))
        }
        StateType::UniqueAsset | StateType::Listing | StateType::Escrow => {
            Ok(StateKey::for_resource(&tenant.0, &resource.0))
        }
    }
}

// ---------------------------------------------------------------------------
// Unique asset (owner/status).
// ---------------------------------------------------------------------------

fn apply_unique_asset(
    before: Option<&StateProjection>,
    operation: &Operation,
    inputs: &BTreeMap<String, Value>,
) -> Result<Value, ExecutorError> {
    let owner = match before {
        Some(current) => state_str(current, "owner")?.to_owned(),
        None => String::new(),
    };
    if operation == asset_op::asset_mint() || operation == asset_op::asset_transfer() {
        let to_owner = input_str(inputs, "to_owner")?;
        Ok(json!({ "owner": to_owner, "status": asset_status::active().as_str() }))
    } else if operation == asset_op::asset_burn() {
        Ok(json!({ "owner": owner, "status": asset_status::burned().as_str() }))
    } else if operation == asset_op::asset_lock() {
        Ok(json!({ "owner": owner, "status": asset_status::locked().as_str() }))
    } else if operation == asset_op::asset_unlock() {
        Ok(json!({ "owner": owner, "status": asset_status::active().as_str() }))
    } else if operation == asset_op::asset_list() {
        Ok(json!({ "owner": owner, "status": asset_status::listed().as_str() }))
    } else if operation == asset_op::asset_delist() {
        Ok(json!({ "owner": owner, "status": asset_status::active().as_str() }))
    } else if operation == asset_op::asset_escrow() {
        Ok(json!({ "owner": owner, "status": asset_status::escrowed().as_str() }))
    } else if operation == asset_op::asset_release() {
        Ok(json!({ "owner": owner, "status": asset_status::active().as_str() }))
    } else if operation == asset_op::asset_redeem() {
        Ok(json!({ "owner": owner, "status": asset_status::redeemed().as_str() }))
    } else if operation == asset_op::asset_restrict() {
        let target = inputs
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("restricted");
        Ok(json!({ "owner": owner, "status": target }))
    } else if operation == asset_op::asset_restore() {
        Ok(json!({ "owner": owner, "status": asset_status::active().as_str() }))
    } else if operation == paid_op::asset_hard_delete() {
        Ok(json!({ "owner": owner, "status": asset_status::tombstoned().as_str() }))
    } else if operation == asset_op::trade_lock() {
        let current = require_current(before, operation)?;
        input_str(inputs, "from_owner")?;
        let trade_id = input_str(inputs, "trade_id")?;
        let current_owner = state_str(current, "owner")?;
        Ok(
            json!({ "owner": current_owner, "status": asset_status::trade_held().as_str(), "trade_id": trade_id }),
        )
    } else if operation == asset_op::trade_unlock() {
        let current = require_current(before, operation)?;
        input_str(inputs, "trade_id")?;
        let current_owner = state_str(current, "owner")?;
        Ok(json!({ "owner": current_owner, "status": asset_status::active().as_str() }))
    } else if operation == asset_op::trade_settle() {
        require_current(before, operation)?;
        input_str(inputs, "from_owner")?;
        let to_owner = input_str(inputs, "to_owner")?;
        input_str(inputs, "trade_id")?;
        Ok(json!({ "owner": to_owner, "status": asset_status::active().as_str() }))
    } else if operation == asset_op::asset_attach_content()
        || operation == asset_op::asset_detach_content()
        || operation == asset_op::asset_update_metadata()
    {
        // Content and metadata attachments mutate profile-defined fields that
        // are opaque to the executor; the state payload is preserved verbatim.
        before.map(|current| current.state.clone()).ok_or_else(|| {
            ExecutorError::TransitionInvalid(String::from(
                "operation requires an existing resource",
            ))
        })
    } else {
        Err(ExecutorError::TransitionInvalid(format!(
            "unknown unique asset operation `{}`",
            operation.as_str()
        )))
    }
}

// ---------------------------------------------------------------------------
// Consumable stack (subject/quantity/unit).
// ---------------------------------------------------------------------------

fn apply_stack(
    before: Option<&StateProjection>,
    operation: &Operation,
    inputs: &BTreeMap<String, Value>,
) -> Result<Value, ExecutorError> {
    if operation == stack_op::stack_create() {
        let subject = input_str(inputs, "subject")?;
        let unit = input_str(inputs, "unit")?;
        let quantity = input_amount(inputs, "quantity")?;
        Ok(json!({
            "subject": subject,
            "quantity": quantity.to_canonical_string(),
            "unit": unit,
        }))
    } else if operation == stack_op::stack_credit() || operation == stack_op::stack_release() {
        let current = require_current(before, operation)?;
        let subject = state_str(current, "subject")?;
        let unit = state_str(current, "unit")?;
        let quantity = state_amount(current, "quantity")?;
        let amount = input_amount(inputs, "quantity")?;
        let next = quantity
            .checked_add(amount)
            .ok_or_else(|| overflow("stack quantity"))?;
        Ok(json!({
            "subject": subject,
            "quantity": next.to_canonical_string(),
            "unit": unit,
        }))
    } else if operation == stack_op::stack_debit()
        || operation == stack_op::stack_consume()
        || operation == stack_op::stack_reserve()
    {
        let current = require_current(before, operation)?;
        let subject = state_str(current, "subject")?;
        let unit = state_str(current, "unit")?;
        let quantity = state_amount(current, "quantity")?;
        let amount = input_amount(inputs, "quantity")?;
        let next = quantity
            .checked_sub(amount)
            .ok_or_else(|| underflow("stack quantity"))?;
        Ok(json!({
            "subject": subject,
            "quantity": next.to_canonical_string(),
            "unit": unit,
        }))
    } else if operation == stack_op::stack_transfer() {
        // `stack.transfer` debits the source stack and preserves its subject.
        // The matching destination credit is computed by
        // [`transfer_after_state`]; the pipeline emits both events so the
        // transfer is an atomic debit + credit pair (§20.5).
        let current = require_current(before, operation)?;
        input_str(inputs, "to_subject")?;
        let subject = state_str(current, "subject")?;
        let unit = state_str(current, "unit")?;
        let quantity = state_amount(current, "quantity")?;
        let amount = input_amount(inputs, "quantity")?;
        let next = quantity
            .checked_sub(amount)
            .ok_or_else(|| underflow("stack quantity"))?;
        Ok(json!({
            "subject": subject,
            "quantity": next.to_canonical_string(),
            "unit": unit,
        }))
    } else if operation == stack_op::stack_adjust() {
        let current = require_current(before, operation)?;
        let subject = state_str(current, "subject")?;
        let unit = state_str(current, "unit")?;
        let quantity = input_amount(inputs, "quantity")?;
        Ok(json!({
            "subject": subject,
            "quantity": quantity.to_canonical_string(),
            "unit": unit,
        }))
    } else if operation == stack_op::stack_expire() {
        let current = require_current(before, operation)?;
        let subject = state_str(current, "subject")?;
        let unit = state_str(current, "unit")?;
        Ok(json!({
            "subject": subject,
            "quantity": "0".to_owned(),
            "unit": unit,
        }))
    } else {
        Err(ExecutorError::TransitionInvalid(format!(
            "unknown consumable stack operation `{}`",
            operation.as_str()
        )))
    }
}

// ---------------------------------------------------------------------------
// Fungible balance (subject/balance/unit).
// ---------------------------------------------------------------------------

fn apply_balance(
    before: Option<&StateProjection>,
    operation: &Operation,
    inputs: &BTreeMap<String, Value>,
) -> Result<Value, ExecutorError> {
    if operation == balance_op::balance_create() {
        let subject = input_str(inputs, "subject")?;
        let unit = input_str(inputs, "unit")?;
        let balance = input_amount(inputs, "balance")?;
        Ok(json!({
            "subject": subject,
            "balance": balance.to_canonical_string(),
            "unit": unit,
        }))
    } else if operation == balance_op::balance_mint()
        || operation == balance_op::balance_credit()
        || operation == balance_op::balance_release()
    {
        let current = require_current(before, operation)?;
        let subject = state_str(current, "subject")?;
        let unit = state_str(current, "unit")?;
        let balance = state_amount(current, "balance")?;
        let amount = input_amount(inputs, "amount")?;
        let next = balance
            .checked_add(amount)
            .ok_or_else(|| overflow("balance"))?;
        Ok(json!({
            "subject": subject,
            "balance": next.to_canonical_string(),
            "unit": unit,
        }))
    } else if operation == balance_op::balance_debit()
        || operation == balance_op::balance_spend()
        || operation == balance_op::balance_reserve()
        || operation == balance_op::balance_burn()
    {
        let current = require_current(before, operation)?;
        let subject = state_str(current, "subject")?;
        let unit = state_str(current, "unit")?;
        let balance = state_amount(current, "balance")?;
        let amount = input_amount(inputs, "amount")?;
        let next = balance
            .checked_sub(amount)
            .ok_or_else(|| underflow("balance"))?;
        Ok(json!({
            "subject": subject,
            "balance": next.to_canonical_string(),
            "unit": unit,
        }))
    } else if operation == balance_op::balance_transfer() {
        // `balance.transfer` debits the source balance and preserves its
        // subject, mirroring the stack transfer rule. The matching destination
        // credit is computed by [`transfer_after_state`] and emitted by the
        // pipeline as the second event of the atomic pair (§20.5).
        let current = require_current(before, operation)?;
        input_str(inputs, "to_subject")?;
        let subject = state_str(current, "subject")?;
        let unit = state_str(current, "unit")?;
        let balance = state_amount(current, "balance")?;
        let amount = input_amount(inputs, "amount")?;
        let next = balance
            .checked_sub(amount)
            .ok_or_else(|| underflow("balance"))?;
        Ok(json!({
            "subject": subject,
            "balance": next.to_canonical_string(),
            "unit": unit,
        }))
    } else if operation == balance_op::balance_convert() {
        let current = require_current(before, operation)?;
        let subject = state_str(current, "subject")?;
        let to_unit = input_str(inputs, "to_unit")?;
        let balance = state_amount(current, "balance")?;
        let amount = input_amount(inputs, "amount")?;
        let next = balance
            .checked_sub(amount)
            .ok_or_else(|| underflow("balance"))?;
        Ok(json!({
            "subject": subject,
            "balance": next.to_canonical_string(),
            "unit": to_unit,
        }))
    } else {
        Err(ExecutorError::TransitionInvalid(format!(
            "unknown fungible balance operation `{}`",
            operation.as_str()
        )))
    }
}

// ---------------------------------------------------------------------------
// Entitlement (subject/status/transferable).
// ---------------------------------------------------------------------------

fn apply_entitlement(
    before: Option<&StateProjection>,
    operation: &Operation,
    inputs: &BTreeMap<String, Value>,
) -> Result<Value, ExecutorError> {
    if operation == entitlement_op::entitlement_grant() {
        let subject = input_str(inputs, "subject")?;
        let transferable = inputs
            .get("transferable")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        Ok(json!({
            "subject": subject,
            "status": entitlement_status::granted().as_str(),
            "transferable": transferable,
        }))
    } else if operation == entitlement_op::entitlement_activate() {
        Ok(entitlement_with_status(
            before,
            operation,
            entitlement_status::active().as_str(),
        )?)
    } else if operation == entitlement_op::entitlement_suspend() {
        Ok(entitlement_with_status(
            before,
            operation,
            entitlement_status::suspended().as_str(),
        )?)
    } else if operation == entitlement_op::entitlement_restore() {
        Ok(entitlement_with_status(
            before,
            operation,
            entitlement_status::active().as_str(),
        )?)
    } else if operation == entitlement_op::entitlement_expire() {
        Ok(entitlement_with_status(
            before,
            operation,
            entitlement_status::expired().as_str(),
        )?)
    } else if operation == entitlement_op::entitlement_revoke() {
        Ok(entitlement_with_status(
            before,
            operation,
            entitlement_status::revoked().as_str(),
        )?)
    } else if operation == entitlement_op::entitlement_transfer() {
        let current = require_current(before, operation)?;
        let to_subject = input_str(inputs, "to_subject")?;
        let status = state_str(current, "status")?;
        let transferable = state_bool(current, "transferable")?;
        Ok(json!({
            "subject": to_subject,
            "status": status,
            "transferable": transferable,
        }))
    } else {
        Err(ExecutorError::TransitionInvalid(format!(
            "unknown entitlement operation `{}`",
            operation.as_str()
        )))
    }
}

/// Builds an entitlement payload with a new status, preserving subject and
/// transferable flag.
///
/// # Errors
///
/// Returns [`ExecutorError::TransitionInvalid`] when the entitlement does not
/// exist or its payload is malformed.
fn entitlement_with_status(
    before: Option<&StateProjection>,
    operation: &Operation,
    status: &str,
) -> Result<Value, ExecutorError> {
    let current = require_current(before, operation)?;
    let subject = state_str(current, "subject")?;
    let transferable = state_bool(current, "transferable")?;
    Ok(json!({
        "subject": subject,
        "status": status,
        "transferable": transferable,
    }))
}

// ---------------------------------------------------------------------------
// Meter (subject/remaining/maximum).
// ---------------------------------------------------------------------------

fn apply_meter(
    before: Option<&StateProjection>,
    operation: &Operation,
    inputs: &BTreeMap<String, Value>,
) -> Result<Value, ExecutorError> {
    if operation == meter_op::meter_create() {
        let subject = input_str(inputs, "subject")?;
        let remaining = input_amount(inputs, "remaining")?;
        let maximum = input_amount(inputs, "maximum")?;
        Ok(json!({
            "subject": subject,
            "remaining": remaining.to_canonical_string(),
            "maximum": maximum.to_canonical_string(),
        }))
    } else if operation == meter_op::meter_consume() {
        let current = require_current(before, operation)?;
        let subject = state_str(current, "subject")?;
        let maximum = state_amount(current, "maximum")?;
        let remaining = state_amount(current, "remaining")?;
        let amount = input_amount(inputs, "amount")?;
        let next = remaining
            .checked_sub(amount)
            .ok_or_else(|| underflow("meter remaining"))?;
        Ok(json!({
            "subject": subject,
            "remaining": next.to_canonical_string(),
            "maximum": maximum.to_canonical_string(),
        }))
    } else if operation == meter_op::meter_refill() {
        let current = require_current(before, operation)?;
        let subject = state_str(current, "subject")?;
        let maximum = state_amount(current, "maximum")?;
        Ok(json!({
            "subject": subject,
            "remaining": maximum.to_canonical_string(),
            "maximum": maximum.to_canonical_string(),
        }))
    } else if operation == meter_op::meter_set_maximum() {
        let current = require_current(before, operation)?;
        let subject = state_str(current, "subject")?;
        let remaining = state_amount(current, "remaining")?;
        let maximum = input_amount(inputs, "maximum")?;
        let clamped = remaining.min(maximum);
        Ok(json!({
            "subject": subject,
            "remaining": clamped.to_canonical_string(),
            "maximum": maximum.to_canonical_string(),
        }))
    } else if operation == meter_op::meter_reset() || operation == meter_op::meter_expire() {
        let current = require_current(before, operation)?;
        let subject = state_str(current, "subject")?;
        let maximum = state_amount(current, "maximum")?;
        Ok(json!({
            "subject": subject,
            "remaining": "0".to_owned(),
            "maximum": maximum.to_canonical_string(),
        }))
    } else {
        Err(ExecutorError::TransitionInvalid(format!(
            "unknown meter operation `{}`",
            operation.as_str()
        )))
    }
}

// ---------------------------------------------------------------------------
// Listing (seller/status).
// ---------------------------------------------------------------------------

fn apply_listing(
    before: Option<&StateProjection>,
    operation: &Operation,
    inputs: &BTreeMap<String, Value>,
) -> Result<Value, ExecutorError> {
    if operation == marketplace_op::listing_create() {
        let seller = input_str(inputs, "seller")?;
        Ok(json!({ "seller": seller, "status": listing_status::listed().as_str() }))
    } else if operation == marketplace_op::listing_cancel() {
        Ok(listing_with_status(
            before,
            operation,
            listing_status::cancelled().as_str(),
        )?)
    } else if operation == marketplace_op::listing_buy() {
        Ok(listing_with_status(
            before,
            operation,
            listing_status::sold().as_str(),
        )?)
    } else if operation == marketplace_op::listing_expire() {
        Ok(listing_with_status(
            before,
            operation,
            listing_status::expired().as_str(),
        )?)
    } else {
        Err(ExecutorError::TransitionInvalid(format!(
            "unknown listing operation `{}`",
            operation.as_str()
        )))
    }
}

/// Builds a listing payload with a new status, preserving the seller.
///
/// # Errors
///
/// Returns [`ExecutorError::TransitionInvalid`] when the listing does not
/// exist or its payload is malformed.
fn listing_with_status(
    before: Option<&StateProjection>,
    operation: &Operation,
    status: &str,
) -> Result<Value, ExecutorError> {
    let current = require_current(before, operation)?;
    let seller = state_str(current, "seller")?;
    Ok(json!({ "seller": seller, "status": status }))
}

// ---------------------------------------------------------------------------
// Escrow (buyer/seller/status).
// ---------------------------------------------------------------------------

fn apply_escrow(
    before: Option<&StateProjection>,
    operation: &Operation,
    inputs: &BTreeMap<String, Value>,
) -> Result<Value, ExecutorError> {
    if operation == marketplace_op::escrow_lock() {
        let buyer = input_str(inputs, "buyer")?;
        let seller = input_str(inputs, "seller")?;
        Ok(json!({
            "buyer": buyer,
            "seller": seller,
            "status": escrow_status::locked().as_str(),
        }))
    } else if operation == marketplace_op::escrow_release() {
        Ok(escrow_with_status(
            before,
            operation,
            escrow_status::released().as_str(),
        )?)
    } else if operation == marketplace_op::escrow_refund() {
        Ok(escrow_with_status(
            before,
            operation,
            escrow_status::refunded().as_str(),
        )?)
    } else {
        Err(ExecutorError::TransitionInvalid(format!(
            "unknown escrow operation `{}`",
            operation.as_str()
        )))
    }
}

/// Builds an escrow payload with a new status, preserving buyer and seller.
///
/// # Errors
///
/// Returns [`ExecutorError::TransitionInvalid`] when the escrow does not exist
/// or its payload is malformed.
fn escrow_with_status(
    before: Option<&StateProjection>,
    operation: &Operation,
    status: &str,
) -> Result<Value, ExecutorError> {
    let current = require_current(before, operation)?;
    let buyer = state_str(current, "buyer")?;
    let seller = state_str(current, "seller")?;
    Ok(json!({
        "buyer": buyer,
        "seller": seller,
        "status": status,
    }))
}

// ---------------------------------------------------------------------------
// Shared helpers.
// ---------------------------------------------------------------------------

/// Resolves the state type a transition applies to.
///
/// Prefers `before.state_type`; for a create (no projection) the state type is
/// inferred from the operation prefix, which uniquely identifies it.
///
/// # Errors
///
/// Returns [`ExecutorError::TransitionInvalid`] when the operation prefix does
/// not identify a known state type.
fn state_type_of(
    before: Option<&StateProjection>,
    operation: &Operation,
) -> Result<StateType, ExecutorError> {
    if let Some(current) = before {
        return Ok(current.state_type);
    }
    // The prefix -> state-type convention lives on the operation newtype; the
    // hint stays open (returns `None` for out-of-convention names) and the
    // executor fails closed for anything it cannot resolve.
    operation.state_type_hint().ok_or_else(|| {
        ExecutorError::TransitionInvalid(format!(
            "cannot determine state type for operation `{}`",
            operation.as_str()
        ))
    })
}

/// Requires the operation to act on an existing resource.
///
/// # Errors
///
/// Returns [`ExecutorError::TransitionInvalid`] when `before` is `None`.
fn require_current<'projection>(
    before: Option<&'projection StateProjection>,
    operation: &Operation,
) -> Result<&'projection StateProjection, ExecutorError> {
    before.ok_or_else(|| {
        ExecutorError::TransitionInvalid(format!(
            "operation `{}` requires an existing resource",
            operation.as_str()
        ))
    })
}

/// Reads a required non-empty string input.
///
/// # Errors
///
/// Returns [`ExecutorError::TransitionInvalid`] when the input is missing, is
/// not a string, or is empty.
fn input_str<'inputs>(
    inputs: &'inputs BTreeMap<String, Value>,
    key: &str,
) -> Result<&'inputs str, ExecutorError> {
    let value = inputs
        .get(key)
        .ok_or_else(|| invalid_input(format!("missing input `{key}`")))?;
    let text = value
        .as_str()
        .ok_or_else(|| invalid_input(format!("input `{key}` must be a string")))?;
    if text.is_empty() {
        return Err(invalid_input(format!("input `{key}` must not be empty")));
    }
    Ok(text)
}

/// Reads a required non-negative integer-string input.
///
/// # Errors
///
/// Returns [`ExecutorError::TransitionInvalid`] when the input is missing or
/// not a canonical non-negative integer string, and fails closed on
/// float-formatted strings.
fn input_amount(inputs: &BTreeMap<String, Value>, key: &str) -> Result<Amount, ExecutorError> {
    let value = inputs
        .get(key)
        .ok_or_else(|| invalid_input(format!("missing input `{key}`")))?;
    parse_amount_str(value, key)
}

/// Reads a string field from a projection's state payload.
///
/// # Errors
///
/// Returns [`ExecutorError::TransitionInvalid`] when the payload has no `key`
/// field or it is not a string.
fn state_str<'projection>(
    projection: &'projection StateProjection,
    key: &str,
) -> Result<&'projection str, ExecutorError> {
    let value = projection
        .state
        .get(key)
        .ok_or_else(|| invalid_input(format!("state payload has no `{key}`")))?;
    value
        .as_str()
        .ok_or_else(|| invalid_input(format!("`{key}` must be a string")))
}

/// Reads a non-negative integer-string field from a projection's state payload.
///
/// # Errors
///
/// Returns [`ExecutorError::TransitionInvalid`] when the payload field is
/// missing or not a canonical non-negative integer string, and fails closed on
/// float-formatted strings.
fn state_amount(projection: &StateProjection, key: &str) -> Result<Amount, ExecutorError> {
    let value = projection
        .state
        .get(key)
        .ok_or_else(|| invalid_input(format!("state payload has no `{key}`")))?;
    parse_amount_str(value, key)
}

/// Reads a boolean field from a projection's state payload.
///
/// # Errors
///
/// Returns [`ExecutorError::TransitionInvalid`] when the payload has no `key`
/// field or it is not a boolean.
fn state_bool(projection: &StateProjection, key: &str) -> Result<bool, ExecutorError> {
    let value = projection
        .state
        .get(key)
        .ok_or_else(|| invalid_input(format!("state payload has no `{key}`")))?;
    value
        .as_bool()
        .ok_or_else(|| invalid_input(format!("`{key}` must be a boolean")))
}

/// Parses a canonical non-negative integer string.
///
/// Float-formatted strings (`.` or exponent markers) are rejected fail-closed
/// (protocol §10.3); anything else that does not parse as a canonical integer is also
/// rejected.
///
/// # Errors
///
/// Returns [`ExecutorError::TransitionInvalid`] for non-string values,
/// float-formatted strings, or strings that do not parse as a canonical integer.
fn parse_amount_str(value: &Value, key: &str) -> Result<Amount, ExecutorError> {
    let text = value
        .as_str()
        .ok_or_else(|| invalid_input(format!("`{key}` must be an integer string")))?;
    if text.is_empty() {
        return Err(invalid_input(format!("`{key}` must be an integer string")));
    }
    if text.contains(['.', 'e', 'E']) {
        return Err(invalid_input(format!(
            "`{key}` must be an integer string (floats are forbidden)"
        )));
    }
    Amount::try_from_str(text)
        .map_err(|_source| invalid_input(format!("`{key}` must be an integer string")))
}

/// Builds a `TransitionInvalid` input error.
const fn invalid_input(message: String) -> ExecutorError {
    ExecutorError::TransitionInvalid(message)
}

/// Builds a checked-add overflow error.
fn overflow(field: &str) -> ExecutorError {
    ExecutorError::TransitionInvalid(format!("`{field}` overflow"))
}

/// Builds a checked-sub underflow error.
fn underflow(field: &str) -> ExecutorError {
    ExecutorError::TransitionInvalid(format!("`{field}` underflow"))
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]
mod tests {
    use super::*;
    use statechronicle_core::digest::ContentDigest;
    use statechronicle_domain::ids::{CommitId, EventId};
    use statechronicle_domain::intent::Operation;

    fn op(name: &str) -> Operation {
        Operation::new(String::from(name)).unwrap()
    }

    fn inputs(entries: &[(&str, Value)]) -> BTreeMap<String, Value> {
        entries
            .iter()
            .map(|(key, value)| (String::from(*key), value.clone()))
            .collect()
    }

    fn projection(state_type: StateType, state: Value) -> StateProjection {
        StateProjection {
            tenant_id: TenantId(String::from("tenant.test")),
            resource_id: ResourceId(String::from("res:test")),
            state_type,
            version: 1,
            last_event_id: EventId::new(String::from("evt_01JZ8X2XRE5ZYW5V9R7VDQBSH4")).unwrap(),
            last_commit_id: CommitId::new(String::from("cmt_01JZ8X5HN3C4PXG5A9FGEWQF5W")).unwrap(),
            state_hash: ContentDigest::new([0u8; 32]),
            state,
        }
    }

    fn asset(status: &str, owner: &str) -> StateProjection {
        projection(
            StateType::UniqueAsset,
            json!({ "owner": owner, "status": status }),
        )
    }

    fn stack(quantity: &str) -> StateProjection {
        projection(
            StateType::ConsumableStack,
            json!({
                "subject": "account:example:player_123",
                "quantity": quantity,
                "unit": "arrows",
            }),
        )
    }

    fn balance(balance: &str) -> StateProjection {
        projection(
            StateType::FungibleBalance,
            json!({
                "subject": "account:example:player_123",
                "balance": balance,
                "unit": "gold_minor",
            }),
        )
    }

    fn meter(remaining: &str, maximum: &str) -> StateProjection {
        projection(
            StateType::MeteredResource,
            json!({
                "subject": "account:example:player_123",
                "remaining": remaining,
                "maximum": maximum,
            }),
        )
    }

    fn entitlement(status: &str, transferable: bool) -> StateProjection {
        projection(
            StateType::Entitlement,
            json!({
                "subject": "account:example:player_123",
                "status": status,
                "transferable": transferable,
            }),
        )
    }

    #[test]
    fn asset_mint_sets_owner_active() {
        let after = apply(
            None,
            &op("asset.mint"),
            &inputs(&[("to_owner", json!("account:example:player_123"))]),
        )
        .unwrap();
        assert_eq!(
            after,
            json!({ "owner": "account:example:player_123", "status": "active" })
        );
    }

    #[test]
    fn asset_transfer_changes_owner() {
        let after = apply(
            Some(&asset("active", "alice")),
            &op("asset.transfer"),
            &inputs(&[("from_owner", json!("alice")), ("to_owner", json!("bob"))]),
        )
        .unwrap();
        assert_eq!(after, json!({ "owner": "bob", "status": "active" }));
    }

    #[test]
    fn asset_lock_unlock_burn_preserve_owner() {
        let active = asset("active", "alice");
        let locked = apply(Some(&active), &op("asset.lock"), &BTreeMap::new()).unwrap();
        assert_eq!(locked, json!({ "owner": "alice", "status": "locked" }));
        let unlocked = apply(
            Some(&asset("locked", "alice")),
            &op("asset.unlock"),
            &BTreeMap::new(),
        )
        .unwrap();
        assert_eq!(unlocked, json!({ "owner": "alice", "status": "active" }));
        let burned = apply(Some(&active), &op("asset.burn"), &BTreeMap::new()).unwrap();
        assert_eq!(burned, json!({ "owner": "alice", "status": "burned" }));
    }

    #[test]
    fn asset_restrict_honors_target_status() {
        let after = apply(
            Some(&asset("active", "alice")),
            &op("asset.restrict"),
            &inputs(&[("status", json!("legal_hold"))]),
        )
        .unwrap();
        assert_eq!(after, json!({ "owner": "alice", "status": "legal_hold" }));
    }

    #[test]
    fn trade_lock_freezes_active_asset_with_trade_id() {
        let after = apply(
            Some(&asset("active", "alice")),
            &op("trade.lock"),
            &inputs(&[
                ("from_owner", json!("alice")),
                ("trade_id", json!("trade_001")),
            ]),
        )
        .unwrap();
        assert_eq!(
            after,
            json!({ "owner": "alice", "status": "trade_held", "trade_id": "trade_001" })
        );
    }

    #[test]
    fn trade_unlock_returns_held_asset_to_active_and_drops_id() {
        let held = projection(
            StateType::UniqueAsset,
            json!({ "owner": "alice", "status": "trade_held", "trade_id": "trade_001" }),
        );
        let after = apply(
            Some(&held),
            &op("trade.unlock"),
            &inputs(&[("trade_id", json!("trade_001"))]),
        )
        .unwrap();
        assert_eq!(after, json!({ "owner": "alice", "status": "active" }));
    }

    #[test]
    fn trade_settle_changes_owner_and_returns_to_active() {
        let held = projection(
            StateType::UniqueAsset,
            json!({ "owner": "alice", "status": "trade_held", "trade_id": "trade_001" }),
        );
        let after = apply(
            Some(&held),
            &op("trade.settle"),
            &inputs(&[
                ("from_owner", json!("alice")),
                ("to_owner", json!("bob")),
                ("trade_id", json!("trade_001")),
            ]),
        )
        .unwrap();
        assert_eq!(after, json!({ "owner": "bob", "status": "active" }));
    }

    #[test]
    fn trade_ops_require_existing_resource_and_inputs() {
        // A trade op on an unborn resource fails closed.
        assert!(matches!(
            apply(None, &op("trade.lock"), &BTreeMap::new()),
            Err(ExecutorError::TransitionInvalid(_))
        ));
        let held = projection(
            StateType::UniqueAsset,
            json!({ "owner": "alice", "status": "trade_held", "trade_id": "trade_001" }),
        );
        // Missing inputs fail closed.
        assert!(matches!(
            apply(Some(&held), &op("trade.settle"), &BTreeMap::new()),
            Err(ExecutorError::TransitionInvalid(_))
        ));
        assert!(matches!(
            apply(Some(&held), &op("trade.unlock"), &BTreeMap::new()),
            Err(ExecutorError::TransitionInvalid(_))
        ));
    }

    #[test]
    fn stack_create_credit_debit_consume() {
        let created = apply(
            None,
            &op("stack.create"),
            &inputs(&[
                ("subject", json!("alice")),
                ("unit", json!("arrows")),
                ("quantity", json!("10")),
            ]),
        )
        .unwrap();
        assert_eq!(
            created,
            json!({ "subject": "alice", "quantity": "10", "unit": "arrows" })
        );

        let credited = apply(
            Some(&stack("10")),
            &op("stack.credit"),
            &inputs(&[("quantity", json!("5"))]),
        )
        .unwrap();
        assert_eq!(credited.get("quantity").unwrap(), "15");

        let debited = apply(
            Some(&stack("10")),
            &op("stack.debit"),
            &inputs(&[("quantity", json!("3"))]),
        )
        .unwrap();
        assert_eq!(debited.get("quantity").unwrap(), "7");

        let consumed = apply(
            Some(&stack("10")),
            &op("stack.consume"),
            &inputs(&[("quantity", json!("10"))]),
        )
        .unwrap();
        assert_eq!(consumed.get("quantity").unwrap(), "0");
    }

    #[test]
    fn stack_debit_underflow_fails_closed() {
        let error = apply(
            Some(&stack("10")),
            &op("stack.debit"),
            &inputs(&[("quantity", json!("11"))]),
        )
        .unwrap_err();
        assert!(matches!(error, ExecutorError::TransitionInvalid(_)));
    }

    #[test]
    fn stack_credit_overflow_fails_closed() {
        let max = u128::MAX.to_string();
        let error = apply(
            Some(&stack(&max)),
            &op("stack.credit"),
            &inputs(&[("quantity", json!("1"))]),
        )
        .unwrap_err();
        assert!(matches!(error, ExecutorError::TransitionInvalid(_)));
    }

    #[test]
    fn stack_release_and_adjust() {
        let released = apply(
            Some(&stack("10")),
            &op("stack.release"),
            &inputs(&[("quantity", json!("2"))]),
        )
        .unwrap();
        assert_eq!(released.get("quantity").unwrap(), "12");

        let adjusted = apply(
            Some(&stack("10")),
            &op("stack.adjust"),
            &inputs(&[("quantity", json!("0"))]),
        )
        .unwrap();
        assert_eq!(adjusted.get("quantity").unwrap(), "0");
    }

    #[test]
    fn balance_credit_debit_spend_convert() {
        let credited = apply(
            Some(&balance("100")),
            &op("balance.credit"),
            &inputs(&[("amount", json!("25"))]),
        )
        .unwrap();
        assert_eq!(credited.get("balance").unwrap(), "125");

        let debited = apply(
            Some(&balance("100")),
            &op("balance.debit"),
            &inputs(&[("amount", json!("40"))]),
        )
        .unwrap();
        assert_eq!(debited.get("balance").unwrap(), "60");

        let spent = apply(
            Some(&balance("100")),
            &op("balance.spend"),
            &inputs(&[("amount", json!("100"))]),
        )
        .unwrap();
        assert_eq!(spent.get("balance").unwrap(), "0");

        let converted = apply(
            Some(&balance("100")),
            &op("balance.convert"),
            &inputs(&[("amount", json!("30")), ("to_unit", json!("silver_minor"))]),
        )
        .unwrap();
        assert_eq!(converted.get("balance").unwrap(), "70");
        assert_eq!(converted.get("unit").unwrap(), "silver_minor");
    }

    #[test]
    fn balance_debit_underflow_fails_closed() {
        let error = apply(
            Some(&balance("10")),
            &op("balance.debit"),
            &inputs(&[("amount", json!("11"))]),
        )
        .unwrap_err();
        assert!(matches!(error, ExecutorError::TransitionInvalid(_)));
    }

    #[test]
    fn balance_credit_overflow_fails_closed() {
        let max = u128::MAX.to_string();
        let error = apply(
            Some(&balance(&max)),
            &op("balance.credit"),
            &inputs(&[("amount", json!("1"))]),
        )
        .unwrap_err();
        assert!(matches!(error, ExecutorError::TransitionInvalid(_)));
    }

    #[test]
    fn meter_consume_refill_set_maximum_reset() {
        let consumed = apply(
            Some(&meter("50", "100")),
            &op("meter.consume"),
            &inputs(&[("amount", json!("30"))]),
        )
        .unwrap();
        assert_eq!(consumed.get("remaining").unwrap(), "20");
        assert_eq!(consumed.get("maximum").unwrap(), "100");

        let refilled = apply(
            Some(&meter("20", "100")),
            &op("meter.refill"),
            &BTreeMap::new(),
        )
        .unwrap();
        assert_eq!(refilled.get("remaining").unwrap(), "100");

        let re_maxed = apply(
            Some(&meter("80", "100")),
            &op("meter.set_maximum"),
            &inputs(&[("maximum", json!("50"))]),
        )
        .unwrap();
        assert_eq!(re_maxed.get("remaining").unwrap(), "50");
        assert_eq!(re_maxed.get("maximum").unwrap(), "50");

        let reset = apply(
            Some(&meter("80", "100")),
            &op("meter.reset"),
            &BTreeMap::new(),
        )
        .unwrap();
        assert_eq!(reset.get("remaining").unwrap(), "0");
    }

    #[test]
    fn meter_consume_underflow_fails_closed() {
        let error = apply(
            Some(&meter("10", "100")),
            &op("meter.consume"),
            &inputs(&[("amount", json!("11"))]),
        )
        .unwrap_err();
        assert!(matches!(error, ExecutorError::TransitionInvalid(_)));
    }

    #[test]
    fn entitlement_lifecycle_and_transfer() {
        let granted = apply(
            None,
            &op("entitlement.grant"),
            &inputs(&[("subject", json!("alice")), ("transferable", json!(true))]),
        )
        .unwrap();
        assert_eq!(granted.get("status").unwrap(), "granted");

        let active = apply(
            Some(&entitlement("granted", true)),
            &op("entitlement.activate"),
            &BTreeMap::new(),
        )
        .unwrap();
        assert_eq!(active.get("status").unwrap(), "active");

        let revoked = apply(
            Some(&entitlement("active", true)),
            &op("entitlement.revoke"),
            &BTreeMap::new(),
        )
        .unwrap();
        assert_eq!(revoked.get("status").unwrap(), "revoked");

        let transferred = apply(
            Some(&entitlement("active", true)),
            &op("entitlement.transfer"),
            &inputs(&[("to_subject", json!("bob"))]),
        )
        .unwrap();
        assert_eq!(transferred.get("subject").unwrap(), "bob");
        assert_eq!(transferred.get("status").unwrap(), "active");
    }

    #[test]
    fn listing_and_escrow_transitions() {
        let listed = apply(
            None,
            &op("listing.create"),
            &inputs(&[("seller", json!("alice"))]),
        )
        .unwrap();
        assert_eq!(listed, json!({ "seller": "alice", "status": "listed" }));

        let sold = apply(
            Some(&projection(
                StateType::Listing,
                json!({ "seller": "alice", "status": "listed" }),
            )),
            &op("listing.buy"),
            &inputs(&[("buyer", json!("bob"))]),
        )
        .unwrap();
        assert_eq!(sold.get("status").unwrap(), "sold");

        let locked = apply(
            None,
            &op("escrow.lock"),
            &inputs(&[("buyer", json!("bob")), ("seller", json!("alice"))]),
        )
        .unwrap();
        assert_eq!(
            locked,
            json!({ "buyer": "bob", "seller": "alice", "status": "locked" })
        );

        let released = apply(
            Some(&projection(
                StateType::Escrow,
                json!({ "buyer": "bob", "seller": "alice", "status": "locked" }),
            )),
            &op("escrow.release"),
            &BTreeMap::new(),
        )
        .unwrap();
        assert_eq!(released.get("status").unwrap(), "released");
    }

    #[test]
    fn floats_are_rejected_fail_closed() {
        let error = apply(
            Some(&stack("10")),
            &op("stack.credit"),
            &inputs(&[("quantity", json!("1.5"))]),
        )
        .unwrap_err();
        assert!(matches!(error, ExecutorError::TransitionInvalid(_)));

        let number_error = apply(
            Some(&stack("10")),
            &op("stack.debit"),
            &inputs(&[("quantity", json!(5))]),
        )
        .unwrap_err();
        assert!(matches!(number_error, ExecutorError::TransitionInvalid(_)));
    }

    #[test]
    fn malformed_and_unknown_inputs_fail_closed() {
        assert!(matches!(
            apply(None, &op("stack.create"), &BTreeMap::new()),
            Err(ExecutorError::TransitionInvalid(_))
        ));
        assert!(matches!(
            apply(Some(&stack("10")), &op("stack.unknown"), &BTreeMap::new()),
            Err(ExecutorError::TransitionInvalid(_))
        ));
        assert!(matches!(
            apply(Some(&stack("10")), &op("asset.mint"), &BTreeMap::new()),
            Err(ExecutorError::TransitionInvalid(_))
        ));
        assert!(matches!(
            apply(None, &op("noop"), &BTreeMap::new()),
            Err(ExecutorError::TransitionInvalid(_))
        ));
    }

    #[test]
    fn state_key_for_matches_accumulator_derivation() {
        let tenant = TenantId(String::from("tenant:acme"));
        let resource = ResourceId(String::from("asset:sword_001"));
        let subject = SubjectId(String::from("account:example:player_123"));

        let resource_key = state_key_for(StateType::UniqueAsset, &tenant, None, &resource).unwrap();
        assert_eq!(
            resource_key,
            StateKey::for_resource("tenant:acme", "asset:sword_001")
        );

        let held_key = state_key_for(
            StateType::FungibleBalance,
            &tenant,
            Some(&subject),
            &resource,
        )
        .unwrap();
        assert_eq!(
            held_key,
            StateKey::for_subject_held(
                "tenant:acme",
                "asset:sword_001",
                "account:example:player_123"
            )
        );

        // Subject-held types require a subject; resource-keyed types reject none.
        assert!(state_key_for(StateType::FungibleBalance, &tenant, None, &resource).is_err());
        assert!(state_key_for(StateType::UniqueAsset, &tenant, Some(&subject), &resource).is_ok());
    }

    #[test]
    fn stack_transfer_credits_destination_with_source_unit() {
        let source = stack("10");
        let destination = stack("20");
        let after = transfer_after_state(
            &source,
            Some(&destination),
            &op("stack.transfer"),
            &inputs(&[("to_subject", json!("bob")), ("quantity", json!("4"))]),
        )
        .unwrap();
        assert_eq!(
            after,
            json!({
                "subject": "bob",
                "quantity": "24",
                "unit": "arrows",
            })
        );
    }

    #[test]
    fn stack_transfer_creates_destination_at_zero_when_absent() {
        let source = stack("10");
        let after = transfer_after_state(
            &source,
            None,
            &op("stack.transfer"),
            &inputs(&[("to_subject", json!("bob")), ("quantity", json!("4"))]),
        )
        .unwrap();
        assert_eq!(
            after,
            json!({
                "subject": "bob",
                "quantity": "4",
                "unit": "arrows",
            })
        );
    }

    #[test]
    fn stack_transfer_overflow_fails_closed() {
        let max = u128::MAX.to_string();
        let destination = projection(
            StateType::ConsumableStack,
            json!({ "subject": "bob", "quantity": max, "unit": "arrows" }),
        );
        let error = transfer_after_state(
            &stack("10"),
            Some(&destination),
            &op("stack.transfer"),
            &inputs(&[("to_subject", json!("bob")), ("quantity", json!("1"))]),
        )
        .unwrap_err();
        assert!(matches!(error, ExecutorError::TransitionInvalid(_)));
    }

    #[test]
    fn balance_transfer_credits_destination_with_source_denomination() {
        let source = balance("100");
        let destination = balance("50");
        let after = transfer_after_state(
            &source,
            Some(&destination),
            &op("balance.transfer"),
            &inputs(&[("to_subject", json!("bob")), ("amount", json!("25"))]),
        )
        .unwrap();
        assert_eq!(
            after,
            json!({
                "subject": "bob",
                "balance": "75",
                "unit": "gold_minor",
            })
        );
    }

    #[test]
    fn balance_transfer_creates_destination_at_zero_when_absent() {
        let source = balance("100");
        let after = transfer_after_state(
            &source,
            None,
            &op("balance.transfer"),
            &inputs(&[("to_subject", json!("bob")), ("amount", json!("25"))]),
        )
        .unwrap();
        assert_eq!(
            after,
            json!({
                "subject": "bob",
                "balance": "25",
                "unit": "gold_minor",
            })
        );
    }

    #[test]
    fn transfer_after_state_rejects_missing_inputs() {
        let source = balance("100");
        assert!(matches!(
            transfer_after_state(
                &source,
                None,
                &op("balance.transfer"),
                &inputs(&[("amount", json!("25"))])
            ),
            Err(ExecutorError::TransitionInvalid(_))
        ));
        assert!(matches!(
            transfer_after_state(
                &source,
                None,
                &op("balance.transfer"),
                &inputs(&[("to_subject", json!("bob"))])
            ),
            Err(ExecutorError::TransitionInvalid(_))
        ));
    }

    #[test]
    fn transfer_after_state_rejects_non_transfer_operations() {
        let source = balance("100");
        for name in ["balance.credit", "stack.credit", "asset.transfer"] {
            assert!(matches!(
                transfer_after_state(
                    &source,
                    None,
                    &op(name),
                    &inputs(&[("to_subject", json!("bob")), ("amount", json!("1"))])
                ),
                Err(ExecutorError::TransitionInvalid(_))
            ));
        }
    }

    #[test]
    fn transfer_after_state_is_deterministic() {
        let source = balance("100");
        let inputs = inputs(&[("to_subject", json!("bob")), ("amount", json!("25"))]);
        let first = transfer_after_state(&source, None, &op("balance.transfer"), &inputs).unwrap();
        let second = transfer_after_state(&source, None, &op("balance.transfer"), &inputs).unwrap();
        assert_eq!(first, second);
    }
}
