//! Profile-owned storage addressing and transition execution policies.

use crate::error::ProfileError;
use statechronicle_domain::event::Event;
use statechronicle_domain::intent::Intent;
use statechronicle_domain::resource::ResourceId;
use statechronicle_domain::resource_state::ResourceState;
use statechronicle_domain::state::StateProjection;
use statechronicle_domain::subject::SubjectId;

/// Address of an additional mutation in a profile's atomic transition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateAddress {
    /// Resource containing the state.
    pub resource: ResourceId,
    /// Optional subject component of the consumer's index key.
    pub subject: Option<SubjectId>,
}

/// Stateless profile semantics used by the domain-independent executor.
/// Defaults describe a single resource-keyed native state. Baseline resource
/// profiles explicitly opt into their ownership, status and transfer policies.
pub trait ExecutionPolicy: Sync {
    /// Checks the event group emitted by a single accepted intent.
    /// # Errors
    /// Returns an error when the group violates the profile's atomic contract.
    fn validate_events(&self, events: &[&Event]) -> Result<(), ProfileError> {
        if events.len() != 1 {
            return Err(ProfileError::AtomicityViolation(String::from(
                "profile requires one event per intent",
            )));
        }
        Ok(())
    }
    /// Subject component for the first index lookup, if any.
    fn primary_subject<'intent>(&self, _intent: &'intent Intent) -> Option<&'intent SubjectId> {
        None
    }
    /// Subject recorded by an existing state, or a profile-specific creation
    /// default. The executor reloads under it when the initial lookup differs.
    fn current_subject(
        &self,
        _intent: &Intent,
        _current: Option<&StateProjection>,
    ) -> Option<SubjectId> {
        None
    }
    /// Applies profile-specific access/availability gates before authority.
    /// # Errors
    /// Returns a profile error when the loaded state cannot accept this request.
    fn check_access(
        &self,
        _intent: &Intent,
        _current: Option<&StateProjection>,
    ) -> Result<(), ProfileError> {
        Ok(())
    }
    /// Validates whether the selected entry point can execute this command.
    /// # Errors
    /// Returns a profile error if the command requires coordination.
    fn validate_route(&self, _intent: &Intent, _coordinated: bool) -> Result<(), ProfileError> {
        Ok(())
    }
    /// Domain's version-zero state commitment before creation.
    /// # Errors
    /// Returns an error if the profile cannot provide an unborn state.
    fn unborn_state(&self, after: &ResourceState) -> Result<ResourceState, ProfileError> {
        let ResourceState::Custom(custom) = after else {
            return Err(ProfileError::InvalidInput(String::from(
                "profile must define its unborn state",
            )));
        };
        let mut unborn = custom.clone();
        unborn.payload.clear();
        Ok(ResourceState::Custom(unborn))
    }
    /// Optional additional state affected by one accepted command.
    /// # Errors
    /// Returns an error for malformed addressing inputs.
    fn companion_target(&self, _intent: &Intent) -> Result<Option<StateAddress>, ProfileError> {
        Ok(None)
    }
    /// Reduces the additional state after its projection has been loaded.
    /// # Errors
    /// Returns a profile error for invalid or unsupported additional mutations.
    fn apply_companion(
        &self,
        _intent: &Intent,
        _primary: &StateProjection,
        _current: Option<&StateProjection>,
    ) -> Result<ResourceState, ProfileError> {
        Err(ProfileError::InvalidInput(String::from(
            "profile has no additional transition",
        )))
    }
}

/// Default policy for resource-keyed native custom domains.
pub struct NativeExecution;
impl ExecutionPolicy for NativeExecution {}
/// Borrowable default without dynamic registration or construction.
pub static NATIVE_EXECUTION: NativeExecution = NativeExecution;
