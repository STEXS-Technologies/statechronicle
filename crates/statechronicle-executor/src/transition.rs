//! Compatibility wrappers for the baseline profile reducers.
use crate::error::ExecutorError;
use serde_json::Value;
use statechronicle_accumulator::key::StateKey;
use statechronicle_domain::{
    intent::Operation, resource::ResourceId, resource_state::ResourceState, state::StateProjection,
    state_type::StateType, subject::SubjectId, tenant::TenantId,
};
use statechronicle_profiles::{error::ProfileError, transition};
use std::collections::BTreeMap;
fn map_error(error: ProfileError) -> ExecutorError {
    match error {
        ProfileError::InvalidInput(message) => ExecutorError::TransitionInvalid(message),
        other @ (ProfileError::UnknownOperation(_)
        | ProfileError::InvalidTransition { .. }
        | ProfileError::NonPositiveAmount
        | ProfileError::InsufficientQuantity { .. }
        | ProfileError::HardDeleteForbidden
        | ProfileError::FloatForbidden
        | ProfileError::OwnershipMismatch { .. }
        | ProfileError::NotTransferable
        | ProfileError::ActorMismatch { .. }
        | ProfileError::ResourceLocked { .. }
        | ProfileError::ValueLegSettleRouting { .. }
        | ProfileError::AtomicityViolation(_)
        | ProfileError::TransferMismatch(_)) => ExecutorError::TransitionInvalid(other.to_string()),
    }
}
/// Applies the baseline reducer. Custom domains use the registered ProfileRules reducer.
/// # Errors
/// Returns a transition error for invalid commands or state.
pub fn apply(
    before: Option<&StateProjection>,
    operation: &Operation,
    inputs: &BTreeMap<String, Value>,
) -> Result<ResourceState, ExecutorError> {
    transition::apply(before, operation, inputs).map_err(map_error)
}
/// Computes the matching destination credit for a baseline transfer.
/// # Errors
/// Returns a transition error for invalid transfers.
pub fn transfer_after_state(
    source: &StateProjection,
    destination: Option<&StateProjection>,
    operation: &Operation,
    inputs: &BTreeMap<String, Value>,
) -> Result<ResourceState, ExecutorError> {
    transition::transfer_after_state(source, destination, operation, inputs).map_err(map_error)
}
/// Derives a state key under the baseline keying conventions.
/// # Errors
/// Returns a transition error when a subject-held state has no subject.
pub fn state_key_for(
    state_type: StateType,
    tenant: &TenantId,
    subject: Option<&SubjectId>,
    resource: &ResourceId,
) -> Result<StateKey, ExecutorError> {
    transition::state_key_for(state_type, tenant, subject, resource).map_err(map_error)
}
