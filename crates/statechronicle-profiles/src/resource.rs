//! Baseline resource execution policies, separate from generic orchestration.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use serde_json::Value;

use crate::consumable_stack::op as stack_op;
use crate::entitlement::op as entitlement_op;
use crate::entitlement::status as entitlement_status;
use crate::execution::{ExecutionPolicy, StateAddress};
use crate::fungible_balance::op as balance_op;
use crate::marketplace::escrow_status;
use crate::marketplace::listing_status;
use crate::marketplace::op as marketplace_op;
use crate::paid_unique_asset::exceptional_status;
use crate::unique_asset::op as asset_op;
use crate::unique_asset::status as asset_status;
use crate::{atomicity, transition};
use statechronicle_core::amount::Amount;
use statechronicle_domain::event::Event;
use statechronicle_domain::intent::{Intent, Operation};
use statechronicle_domain::resource_state::{
    ConsumableStackState, EntitlementState, EscrowState, FungibleBalanceState, ListingState,
    MeterState, ResourceState, UniqueAssetState,
};
use statechronicle_domain::state::StateProjection;
use statechronicle_domain::state_type::StateType;
use statechronicle_domain::status::Status;
use statechronicle_domain::subject::SubjectId;

use crate::error::ProfileError;

/// Builds a lazily-initialized `&'static [Operation]` slice from accessors.
macro_rules! op_vec {
    ($( $op:expr ),* $(,)?) => {{
        static ALL: OnceLock<Vec<Operation>> = OnceLock::new();
        ALL.get_or_init(|| vec![ $( $op.to_owned(), )* ])
    }};
}

/// Builds a lazily-initialized `&'static [Status]` slice from accessors.
macro_rules! status_vec {
    ($( $st:expr ),* $(,)?) => {{
        static ALL: OnceLock<Vec<Status>> = OnceLock::new();
        ALL.get_or_init(|| vec![ $( $st.to_owned(), )* ])
    }};
}

/// Returns the always-available (free) statuses: no operation is blocked from
/// them.
fn free_statuses() -> &'static [Status] {
    status_vec![asset_status::active(), entitlement_status::granted()]
}

/// Returns the terminal statuses: no operation may ever leave them
/// (protocol §20.1).
fn terminal_statuses() -> &'static [Status] {
    status_vec![
        asset_status::burned(),
        listing_status::expired(),
        asset_status::tombstoned(),
        entitlement_status::revoked(),
        asset_status::redeemed(),
        listing_status::sold(),
        listing_status::cancelled(),
        escrow_status::released(),
        escrow_status::refunded(),
    ]
}

/// Returns the exceptional statuses a paid unique asset may be restricted
/// into; only `asset.restore` escapes.
fn exceptional_statuses() -> &'static [Status] {
    status_vec![
        asset_status::restricted(),
        asset_status::quarantined(),
        asset_status::unsupported(),
        exceptional_status::legal_hold(),
        exceptional_status::fraud_lock(),
        exceptional_status::policy_restricted(),
    ]
}

/// Checks that the operation's `from_owner` input matches the resource's
/// current owner or holder (protocol §18.2 "Current owner, holder, balance,
/// quantity, or subject does not match operation input").
///
/// The owner identity is read from the projection's `owner` field, falling
/// back to `subject` for subject-held types. When the operation supplies no
/// `from_owner` input, the check passes: the profile's own rules decide
/// whether ownership consent is required. The `_intent` parameter is reserved
/// for pipeline-level context (the resource and actor are already bound into
/// the intent) and currently carries no additional gate.
///
/// # Errors
///
/// Returns [`ProfileError::ActorMismatch`] when `from_owner` is present and
/// differs from the current owner or holder.
pub fn check_owner(
    _intent: &Intent,
    current: Option<&StateProjection>,
    inputs: &BTreeMap<String, Value>,
) -> Result<(), ProfileError> {
    let Some(projection) = current else {
        return Ok(());
    };
    let Some(actual) = inputs.get("from_owner").and_then(Value::as_str) else {
        return Ok(());
    };
    let Some(expected) = projection
        .state
        .owner()
        .or_else(|| projection.state.subject())
        .map(|subject| subject.0.as_str())
    else {
        return Ok(());
    };
    if actual != expected {
        return Err(ProfileError::ActorMismatch {
            expected: String::from(expected),
            actual: String::from(actual),
        });
    }
    Ok(())
}

/// Checks that the resource's status does not block the operation
/// (protocol §18.2 "Resource is locked, burned, revoked, or escrowed in a way
/// that blocks the operation").
///
/// The gate is fail-closed and mirrors the profiles' status conventions
/// (`statechronicle-profiles` §20): terminal statuses block every operation;
/// non-terminal blocking statuses (locked, escrowed, listed, suspended,
/// exceptional) block every operation except the status's own escape
/// operations; payloads without a `status` field (stacks, balances, meters)
/// are governed entirely by the profile's quantity rules and pass.
///
/// # Errors
///
/// Returns [`ProfileError::ResourceLocked`] when the resource's status blocks
/// the operation or the status is unrecognized (fail closed).
pub fn check_resource_availability(
    current: &StateProjection,
    operation: &Operation,
) -> Result<(), ProfileError> {
    let Some(status_str) = current.state.status().map(Status::as_str) else {
        return Ok(());
    };
    // An unrecognized status string fails closed: it is treated as locked so
    // no operation proceeds against an unknown state.
    let Ok(status) = Status::try_from_str(status_str) else {
        return Err(blocked(current));
    };
    if free_statuses().contains(&status) {
        return Ok(());
    }
    if terminal_statuses().contains(&status) {
        return Err(blocked(current));
    }
    if exceptional_statuses().contains(&status) && operation == asset_op::asset_restore() {
        return Ok(());
    }
    let escapes = status_escapes(&status);
    if escapes.contains(operation) {
        return Ok(());
    }
    Err(blocked(current))
}

/// Returns the escape operations permitted from a non-terminal blocking status.
///
/// The `listed` and `locked` statuses are shared between unique assets and
/// marketplace resources (a listing is `listed`, an escrow is `locked`), so
/// each shared status also admits the marketplace escape operations. This is
/// safe because the profile layer is state-type-scoped: a unique asset in
/// `listed` status can never execute `listing.buy` (unknown operation for the
/// unique-asset profile), and a listing can never execute `asset.delist`.
fn status_escapes(status: &Status) -> &'static [Operation] {
    if *status == *asset_status::locked() {
        op_vec![
            asset_op::asset_unlock(),
            asset_op::asset_restrict(),
            marketplace_op::escrow_release(),
            marketplace_op::escrow_refund(),
        ]
    } else if *status == *asset_status::escrowed() {
        op_vec![asset_op::asset_release(), asset_op::asset_restrict()]
    } else if *status == *asset_status::trade_held() {
        op_vec![
            asset_op::trade_unlock(),
            asset_op::trade_settle(),
            asset_op::asset_restrict(),
        ]
    } else if *status == *asset_status::listed() {
        op_vec![
            asset_op::asset_delist(),
            asset_op::asset_redeem(),
            asset_op::asset_restrict(),
            marketplace_op::listing_buy(),
            marketplace_op::listing_cancel(),
            marketplace_op::listing_expire(),
        ]
    } else if *status == *entitlement_status::suspended() {
        op_vec![
            entitlement_op::entitlement_restore(),
            entitlement_op::entitlement_expire(),
        ]
    } else if *status == *entitlement_status::granted() {
        op_vec![
            entitlement_op::entitlement_activate(),
            entitlement_op::entitlement_expire(),
            entitlement_op::entitlement_revoke(),
            entitlement_op::entitlement_transfer(),
        ]
    } else {
        // Unreachable for terminal/exceptional/free statuses; fail closed by
        // returning no escapes so any operation is blocked.
        &[]
    }
}

/// Builds a fail-closed `ResourceLocked` error for a blocked operation.
fn blocked(current: &StateProjection) -> ProfileError {
    ProfileError::ResourceLocked {
        resource: current.resource_id.0.clone(),
    }
}

/// Returns the subject used for the initial state-index lookup of a
/// subject-held resource.
///
/// The acting actor is the creator default (Gap 3): it is the key used to
/// *query* the index. Once a resource exists, the projection's own `subject`
/// field is the authoritative holder (see [`holder_for`]), and step 6a
/// re-resolves under it when it differs. Owner-based types
/// ([`StateType::UniqueAsset`], [`StateType::Listing`], [`StateType::Escrow`])
/// carry no subject in their key.
pub const fn subject_for(intent: &Intent, state_type: StateType) -> Option<&SubjectId> {
    match state_type {
        StateType::ConsumableStack
        | StateType::FungibleBalance
        | StateType::Entitlement
        | StateType::MeteredResource => Some(&intent.actor),
        StateType::UniqueAsset | StateType::Listing | StateType::Escrow | StateType::Custom => None,
    }
}

fn empty_state_for(state_type: StateType) -> Result<ResourceState, ProfileError> {
    let subject = SubjectId(String::new());
    let status = Status::from_static("active");
    Ok(match state_type {
        StateType::UniqueAsset => ResourceState::UniqueAsset(UniqueAssetState {
            owner: subject,
            status,
            trade_id: None,
        }),
        StateType::ConsumableStack => ResourceState::ConsumableStack(ConsumableStackState {
            subject,
            quantity: Amount::ZERO,
            unit: String::new(),
        }),
        StateType::FungibleBalance => ResourceState::FungibleBalance(FungibleBalanceState {
            subject,
            balance: Amount::ZERO,
            unit: String::new(),
        }),
        StateType::Entitlement => ResourceState::Entitlement(EntitlementState {
            subject,
            status,
            transferable: false,
        }),
        StateType::MeteredResource => ResourceState::MeteredResource(MeterState {
            subject,
            remaining: Amount::ZERO,
            maximum: Amount::ZERO,
        }),
        StateType::Listing => ResourceState::Listing(ListingState {
            seller: subject,
            status,
        }),
        StateType::Custom => {
            return Err(ProfileError::InvalidInput(String::from(
                "custom unborn state requires domain/schema",
            )));
        }
        StateType::Escrow => ResourceState::Escrow(EscrowState {
            buyer: subject.clone(),
            seller: subject,
            status,
        }),
    })
}

/// Resolves the authoritative holder of a subject-held resource
/// (protocol §9/§10).
///
/// The rule: a projection's `subject` field is the source of truth once a
/// resource exists; the acting actor is only the creator default used when no
/// projection exists yet (the create path). Returns `None` for owner-based
/// state types, which key by resource alone.
#[allow(clippy::collapsible_if)]
pub fn holder_for(
    current: Option<&StateProjection>,
    intent: &Intent,
    state_type: StateType,
) -> Option<SubjectId> {
    match state_type {
        StateType::ConsumableStack
        | StateType::FungibleBalance
        | StateType::Entitlement
        | StateType::MeteredResource => {
            if let Some(projection) = current {
                if let Some(subject) = projection.state.subject().map(|s| s.0.as_str()) {
                    if !subject.is_empty() {
                        return Some(SubjectId(String::from(subject)));
                    }
                }
            }
            Some(intent.actor.clone())
        }
        StateType::UniqueAsset | StateType::Listing | StateType::Escrow | StateType::Custom => None,
    }
}

/// Returns whether the operation is a subject-held atomic transfer
/// (stack.transfer / balance.transfer) that requires a destination credit.
pub fn is_transfer_operation(operation: &Operation) -> bool {
    operation == stack_op::stack_transfer() || operation == balance_op::balance_transfer()
}

/// Reads the destination holder (`to_subject`) from a transfer's inputs.
///
/// # Errors
///
/// Returns [`ProfileError::InvalidInput`] when `to_subject` is missing
/// or empty.
pub fn destination_holder(inputs: &BTreeMap<String, Value>) -> Result<SubjectId, ProfileError> {
    let subject = inputs
        .get("to_subject")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ProfileError::InvalidInput(String::from("missing input `to_subject`")))?;
    Ok(SubjectId(String::from(subject)))
}

impl ExecutionPolicy for ResourceExecution {
    fn validate_events(&self, events: &[&Event]) -> Result<(), ProfileError> {
        if events.len() == 1 {
            return Ok(());
        }
        let group: Vec<Event> = events.iter().map(|event| (*event).clone()).collect();
        atomicity::validate_batch_consistency(&group)
    }
    fn primary_subject<'intent>(&self, intent: &'intent Intent) -> Option<&'intent SubjectId> {
        subject_for(intent, self.state_type)
    }
    fn current_subject(
        &self,
        intent: &Intent,
        current: Option<&StateProjection>,
    ) -> Option<SubjectId> {
        holder_for(current, intent, self.state_type)
    }
    fn check_access(
        &self,
        intent: &Intent,
        current: Option<&StateProjection>,
    ) -> Result<(), ProfileError> {
        check_owner(intent, current, &intent.inputs)?;
        if let Some(state) = current {
            check_resource_availability(state, &intent.operation)?;
        }
        Ok(())
    }
    fn validate_route(&self, intent: &Intent, coordinated: bool) -> Result<(), ProfileError> {
        if !coordinated
            && intent.operation == *asset_op::trade_settle()
            && atomicity::declares_value_leg(intent)
        {
            return Err(ProfileError::ValueLegSettleRouting {
                intent_id: intent.intent_id.0.clone(),
            });
        }
        Ok(())
    }
    fn unborn_state(&self, _after: &ResourceState) -> Result<ResourceState, ProfileError> {
        empty_state_for(self.state_type)
    }
    fn companion_target(&self, intent: &Intent) -> Result<Option<StateAddress>, ProfileError> {
        if !is_transfer_operation(&intent.operation) {
            return Ok(None);
        }
        Ok(Some(StateAddress {
            resource: intent.resource_id.clone(),
            subject: Some(destination_holder(&intent.inputs)?),
        }))
    }
    fn apply_companion(
        &self,
        intent: &Intent,
        primary: &StateProjection,
        current: Option<&StateProjection>,
    ) -> Result<ResourceState, ProfileError> {
        transition::transfer_after_state(primary, current, &intent.operation, &intent.inputs)
    }
}
/// Baseline resource semantics selected explicitly by each resource profile.
pub struct ResourceExecution {
    state_type: StateType,
}
/// Resolves a statically composed baseline resource policy.
pub fn policy(state_type: StateType) -> &'static dyn ExecutionPolicy {
    static UNIQUEASSET: ResourceExecution = ResourceExecution {
        state_type: StateType::UniqueAsset,
    };
    static CONSUMABLESTACK: ResourceExecution = ResourceExecution {
        state_type: StateType::ConsumableStack,
    };
    static FUNGIBLEBALANCE: ResourceExecution = ResourceExecution {
        state_type: StateType::FungibleBalance,
    };
    static ENTITLEMENT: ResourceExecution = ResourceExecution {
        state_type: StateType::Entitlement,
    };
    static METEREDRESOURCE: ResourceExecution = ResourceExecution {
        state_type: StateType::MeteredResource,
    };
    static LISTING: ResourceExecution = ResourceExecution {
        state_type: StateType::Listing,
    };
    static ESCROW: ResourceExecution = ResourceExecution {
        state_type: StateType::Escrow,
    };
    match state_type {
        StateType::UniqueAsset => &UNIQUEASSET,
        StateType::ConsumableStack => &CONSUMABLESTACK,
        StateType::FungibleBalance => &FUNGIBLEBALANCE,
        StateType::Entitlement => &ENTITLEMENT,
        StateType::MeteredResource => &METEREDRESOURCE,
        StateType::Listing => &LISTING,
        StateType::Escrow => &ESCROW,
        StateType::Custom => &crate::execution::NATIVE_EXECUTION,
    }
}
