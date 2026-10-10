//! Typed consumer profiles using canonical BCS payloads.
use crate::error::ProfileError;
use crate::registry::ProfileRules;
use bcs::{from_bytes, serialize_into};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, from_value};
use statechronicle_core::canonicalize::canonicalize_and_digest;
use statechronicle_core::limits::MAX_INTENT_BYTES;
use statechronicle_domain::commit::ProfileId;
use statechronicle_domain::custom::CustomState;
use statechronicle_domain::event::{EVENT_SCHEMA, Event};
use statechronicle_domain::intent::{Intent, Operation};
use statechronicle_domain::resource_state::ResourceState;
use statechronicle_domain::state::StateProjection;
use statechronicle_domain::state_type::StateType;
use statechronicle_profile::StateProfile;
use std::collections::BTreeMap;
use std::fmt::Display;
use std::io::{Error as IoError, ErrorKind, Result as IoResult, Write};

/// Adapts a native profile into the shared execution/commit/proof pipeline.
/// Commands are BCS bytes in `inputs.command`; operation names select the
/// profile's public API and remain available to authority evaluators. The
/// profile's command type must encode its transition discriminator itself.
/// Authority bindings are mandatory by default. Override via `with_optional_authority`
/// only when the consumer's submission policy supplies an equivalent boundary.
pub struct TypedProfile<P> {
    profile: P,
    operations: &'static [Operation],
    authority_required: bool,
}

impl<P> TypedProfile<P> {
    /// Constructs a statically composable profile adapter.
    pub const fn new(profile: P, operations: &'static [Operation]) -> Self {
        Self {
            profile,
            operations,
            authority_required: true,
        }
    }
    /// Allows the consumer to accept commands without an authority binding.
    pub const fn with_optional_authority(mut self) -> Self {
        self.authority_required = false;
        self
    }
}

fn invalid(error: impl Display) -> ProfileError {
    ProfileError::InvalidInput(error.to_string())
}

impl<P> TypedProfile<P>
where
    P: StateProfile,
    P::State: Serialize + DeserializeOwned,
    P::Command: Serialize + DeserializeOwned,
    P::Error: Display,
{
    fn identity(&self) -> Result<ProfileId, ProfileError> {
        ProfileId::new(String::from(P::DOMAIN_ID)).map_err(invalid)
    }

    fn bound(&self, payload: &[u8]) -> Result<(), ProfileError> {
        self.check_size(payload.len())
    }

    fn check_size(&self, size: usize) -> Result<(), ProfileError> {
        if P::SCHEMA_VERSION == 0
            || P::MAX_PAYLOAD_BYTES == 0
            || size > P::MAX_PAYLOAD_BYTES.min(MAX_INTENT_BYTES)
        {
            return Err(invalid("invalid schema or payload size limit"));
        }
        Ok(())
    }

    fn encode<T: Serialize>(&self, value: &T) -> Result<Vec<u8>, ProfileError> {
        self.check_size(0)?;
        let mut buffer = BoundedBuffer {
            bytes: Vec::new(),
            limit: P::MAX_PAYLOAD_BYTES.min(MAX_INTENT_BYTES),
        };
        serialize_into(&mut buffer, value).map_err(invalid)?;
        Ok(buffer.bytes)
    }

    /// Decodes and validates state after checking domain, schema, size and canonicality.
    /// This checks domain semantics; callers must also authenticate the enclosing
    /// proof or commit before trusting the supplied state.
    /// # Errors
    /// Rejects other domains/schemas, malformed BCS, and invalid state invariants.
    pub fn decode_state(&self, state: &ResourceState) -> Result<P::State, ProfileError> {
        let ResourceState::Custom(custom) = state else {
            return Err(invalid("custom state required"));
        };
        if custom.domain != self.identity()? || custom.schema_version != P::SCHEMA_VERSION {
            return Err(invalid("domain/schema mismatch"));
        }
        self.bound(&custom.payload)?;
        let decoded: P::State = from_bytes(&custom.payload).map_err(invalid)?;
        if self.encode(&decoded)? != custom.payload {
            return Err(invalid("noncanonical state"));
        }
        self.profile.validate_state(&decoded).map_err(invalid)?;
        Ok(decoded)
    }

    /// Recomputes a committed transition from its accepted intent and prior state.
    /// Historical signature, authority and chain verification are separate caller
    /// responsibilities; this verifies deterministic domain semantics and commitments.
    /// # Errors
    /// Rejects scope, identity, version, before/after hash or reduction mismatches.
    pub fn verify_transition(
        &self,
        current: Option<&StateProjection>,
        intent: &Intent,
        event: &Event,
    ) -> Result<(), ProfileError> {
        if event.schema != EVENT_SCHEMA
            || event.tenant_id != intent.tenant_id
            || event.resource_id != intent.resource_id
            || event.intent_id != intent.intent_id
            || event.actor != intent.actor
            || event.operation != intent.operation
            || intent.state_type != Some(StateType::Custom)
        {
            return Err(invalid("transition identity mismatch"));
        }
        let version = current.map_or(0, |projection| projection.version);
        if current.is_some_and(|projection| projection.version == 0)
            || intent.expected_version != version
            || event.before.version != version
            || event.after.version
                != version
                    .checked_add(1)
                    .ok_or_else(|| invalid("version overflow"))?
        {
            return Err(invalid("transition version mismatch"));
        }
        let after = self.apply(current, &intent.operation, &intent.inputs)?;
        let before = match current {
            Some(projection) => {
                if projection.tenant_id != intent.tenant_id
                    || projection.resource_id != intent.resource_id
                {
                    return Err(invalid("prior state scope mismatch"));
                }
                if canonicalize_and_digest(&projection.state).map_err(invalid)?
                    != projection.state_hash
                {
                    return Err(invalid("prior state hash mismatch"));
                }
                projection.state.clone()
            }
            None => {
                let ResourceState::Custom(mut unborn) = after.clone() else {
                    return Err(invalid("custom state required"));
                };
                unborn.payload.clear();
                ResourceState::Custom(unborn)
            }
        };
        if event.before.state != before
            || event.after.state != after
            || canonicalize_and_digest(&before).map_err(invalid)? != event.before.state_hash
            || canonicalize_and_digest(&after).map_err(invalid)? != event.after.state_hash
        {
            return Err(invalid("transition commitment mismatch"));
        }
        Ok(())
    }

    fn arguments(
        &self,
        operation: &Operation,
        current: Option<&StateProjection>,
        inputs: &BTreeMap<String, Value>,
    ) -> Result<(Option<P::State>, P::Command), ProfileError> {
        if !self.operations.contains(operation) {
            return Err(ProfileError::UnknownOperation(operation.0.clone()));
        }
        let domain = inputs
            .get("profile")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid("missing profile"))?;
        let id = ProfileId::new(String::from(domain)).map_err(invalid)?;
        if id != self.identity()?
            || inputs.get("schema_version").and_then(Value::as_u64)
                != Some(u64::from(P::SCHEMA_VERSION))
        {
            return Err(invalid("domain/schema mismatch"));
        }
        let command_value = inputs
            .get("command")
            .ok_or_else(|| invalid("missing command"))?;
        let command_array = command_value
            .as_array()
            .ok_or_else(|| invalid("command must be a BCS byte array"))?;
        self.check_size(command_array.len())?;
        let command_bytes: Vec<u8> = from_value(command_value.clone()).map_err(invalid)?;
        self.bound(&command_bytes)?;
        let command: P::Command = from_bytes(&command_bytes).map_err(invalid)?;
        if self.encode(&command)? != command_bytes {
            return Err(invalid("noncanonical command"));
        }
        let command_operation =
            Operation::new(String::from(self.profile.operation(&command))).map_err(invalid)?;
        if &command_operation != operation {
            return Err(invalid("command operation mismatch"));
        }
        let state = current
            .map(|projection| {
                if projection.state_type != StateType::Custom {
                    return Err(invalid("projection state type mismatch"));
                }
                self.decode_state(&projection.state)
            })
            .transpose()?;
        Ok((state, command))
    }
}

impl<P> ProfileRules for TypedProfile<P>
where
    P: StateProfile,
    P::State: Serialize + DeserializeOwned,
    P::Command: Serialize + DeserializeOwned,
    P::Error: Display,
{
    fn state_type(&self) -> StateType {
        StateType::Custom
    }
    fn profile_id(&self) -> &'static str {
        P::DOMAIN_ID
    }
    fn schema_version(&self) -> u32 {
        P::SCHEMA_VERSION
    }
    fn validate_registration(&self) -> Result<(), ProfileError> {
        self.check_size(0)
    }
    fn allowed_operations(&self) -> &'static [Operation] {
        self.operations
    }
    fn requires_authority(&self, _operation: &Operation) -> bool {
        self.authority_required
    }
    fn check(
        &self,
        operation: &Operation,
        current: Option<&StateProjection>,
        inputs: &BTreeMap<String, Value>,
    ) -> Result<(), ProfileError> {
        let (state, command) = self.arguments(operation, current, inputs)?;
        self.profile
            .validate(state.as_ref(), &command)
            .map_err(invalid)
    }
    fn apply(
        &self,
        current: Option<&StateProjection>,
        operation: &Operation,
        inputs: &BTreeMap<String, Value>,
    ) -> Result<ResourceState, ProfileError> {
        let (state, command) = self.arguments(operation, current, inputs)?;
        self.profile
            .validate(state.as_ref(), &command)
            .map_err(invalid)?;
        let next = self
            .profile
            .apply(state.as_ref(), &command)
            .map_err(invalid)?;
        self.profile.validate_state(&next).map_err(invalid)?;
        let payload = self.encode(&next)?;
        self.bound(&payload)?;
        Ok(ResourceState::Custom(CustomState {
            domain: self.identity()?,
            schema_version: P::SCHEMA_VERSION,
            payload,
        }))
    }
}

/// Prevents oversized serialized outputs from growing an unbounded byte buffer.
struct BoundedBuffer {
    bytes: Vec<u8>,
    limit: usize,
}
impl Write for BoundedBuffer {
    fn write(&mut self, bytes: &[u8]) -> IoResult<usize> {
        if bytes.len() > self.limit.saturating_sub(self.bytes.len()) {
            return Err(IoError::new(ErrorKind::InvalidData, "payload size limit"));
        }
        self.bytes
            .try_reserve_exact(bytes.len())
            .map_err(|error| IoError::other(error.to_string()))?;
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> IoResult<()> {
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;
    #[test]
    fn bounded_writer_rejects_growth_without_partial_write() {
        let mut buffer = BoundedBuffer {
            bytes: Vec::new(),
            limit: 3,
        };
        buffer.write_all(&[1, 2]).unwrap();
        assert!(buffer.write_all(&[3, 4]).is_err());
        assert_eq!(buffer.bytes, vec![1, 2]);
    }
    struct Counter;
    impl StateProfile for Counter {
        type State = u64;
        type Command = u64;
        type Error = &'static str;
        const DOMAIN_ID: &'static str = "test.counter";
        const SCHEMA_VERSION: u32 = 1;
        const MAX_PAYLOAD_BYTES: usize = 8;
        fn operation(&self, _command: &u64) -> &'static str {
            "counter.set"
        }
        fn validate(&self, _current: Option<&u64>, _command: &u64) -> Result<(), Self::Error> {
            Ok(())
        }
        fn apply(&self, _current: Option<&u64>, command: &u64) -> Result<u64, Self::Error> {
            Ok(*command)
        }
        fn validate_state(&self, state: &u64) -> Result<(), Self::Error> {
            if *state > 100 {
                Err("counter limit")
            } else {
                Ok(())
            }
        }
    }
    #[test]
    fn canonical_decoder_checks_identity_and_invariants() {
        let profile = TypedProfile::new(Counter, &[]);
        let mut state = ResourceState::Custom(CustomState {
            domain: ProfileId::new(String::from("test.counter")).unwrap(),
            schema_version: 1,
            payload: 42u64.to_le_bytes().to_vec(),
        });
        assert_eq!(profile.decode_state(&state).unwrap(), 42);
        if let ResourceState::Custom(custom) = &mut state {
            custom.payload.push(0);
        }
        assert!(profile.decode_state(&state).is_err());
        if let ResourceState::Custom(custom) = &mut state {
            custom.payload = 101u64.to_le_bytes().to_vec();
        }
        assert!(profile.decode_state(&state).is_err());
        if let ResourceState::Custom(custom) = &mut state {
            custom.payload = 42u64.to_le_bytes().to_vec();
            custom.schema_version = 2;
        }
        assert!(profile.decode_state(&state).is_err());
    }
}
