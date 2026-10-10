//! Native profile reduction with bounded canonical input/output.
//!
//! This is the pure transition stage. Authentication, historical authority,
//! persistence, idempotency and event/commit formation remain caller facilities.

use crate::codec::{CanonicalCodec, CodecError, Decoder, Encoder};
use crate::integrity::custom_state_hash;
use statechronicle_profile::StateProfile;

/// Borrowed, versioned native state encoding.
#[derive(Debug, Clone, Copy)]
pub struct EncodedState<'input> {
    /// Stable domain identity.
    pub domain: &'input str,
    /// Immutable schema version.
    pub schema: u32,
    /// Current state version; live states start at one.
    pub version: u64,
    /// Canonical native payload, excluding the domain envelope.
    pub payload: &'input [u8],
}
/// Transition inputs supplied by an authenticated caller.
pub struct TransitionRequest<'input> {
    /// Signed/requested profile domain.
    pub domain: &'input str,
    /// Signed/requested schema version.
    pub schema: u32,
    /// Expected current version; creation uses zero.
    pub expected_version: u64,
    /// Signed operation bound to the command discriminator.
    pub operation: &'input str,
    /// Canonical native command encoding.
    pub command: &'input [u8],
    /// Current canonical state, or absence for creation.
    pub current: Option<EncodedState<'input>>,
}
/// Successful reduction metadata. The output payload occupies the first
/// `payload_len` bytes of the caller's output buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransitionResult {
    /// Produced native payload length.
    pub payload_len: usize,
    /// Version after the transition.
    pub version: u64,
    /// Digest of the complete tagged custom-state envelope.
    pub state_hash: [u8; 32],
}
/// Pure transition failures. Consumer errors require no formatting or allocation.
#[derive(Debug, PartialEq, Eq)]
pub enum TransitionError<DomainError> {
    /// Requested domain/schema is invalid or differs from the profile/state.
    Identity,
    /// The expected version is stale, invalid or cannot be incremented.
    Version,
    /// Command discriminator does not match the requested operation.
    Operation,
    /// Encoding is malformed, noncanonical or exceeds a caller/profile bound.
    Codec(CodecError),
    /// Consumer validation or reduction failed.
    Domain(DomainError),
}

/// Reduces one profile using caller-owned output and canonicalization scratch.
/// No library allocation occurs. Consumers must keep their native model,
/// reducer and codec bounded as well. On any error, discard the output buffer;
/// input state is never mutated and no durable effects occur.
///
/// # Errors
/// Rejects wrong identity/schema, stale versions, operation substitution,
/// invalid/canonicalization-mismatched encodings and consumer invariant errors.
pub fn execute<P>(
    profile: &P,
    request: &TransitionRequest<'_>,
    output: &mut [u8],
    scratch: &mut [u8],
) -> Result<TransitionResult, TransitionError<P::Error>>
where
    P: StateProfile,
    P::State: CanonicalCodec,
    P::Command: CanonicalCodec,
{
    if P::DOMAIN_ID.is_empty()
        || P::DOMAIN_ID.chars().take(129).count() > 128
        || P::DOMAIN_ID.chars().any(char::is_control)
        || P::SCHEMA_VERSION == 0
        || P::MAX_PAYLOAD_BYTES == 0
        || request.domain != P::DOMAIN_ID
        || request.schema != P::SCHEMA_VERSION
    {
        return Err(TransitionError::Identity);
    }
    let current_version = request.current.map_or(0, |state| state.version);
    if current_version != request.expected_version
        || request.current.is_some_and(|state| state.version == 0)
    {
        return Err(TransitionError::Version);
    }
    let version = current_version
        .checked_add(1)
        .ok_or(TransitionError::Version)?;
    let command: P::Command = decode_canonical(request.command, P::MAX_PAYLOAD_BYTES, scratch)
        .map_err(TransitionError::Codec)?;
    if profile.operation(&command) != request.operation {
        return Err(TransitionError::Operation);
    }
    let current = match request.current {
        Some(state) => {
            if state.domain != P::DOMAIN_ID || state.schema != P::SCHEMA_VERSION {
                return Err(TransitionError::Identity);
            }
            let decoded: P::State = decode_canonical(state.payload, P::MAX_PAYLOAD_BYTES, scratch)
                .map_err(TransitionError::Codec)?;
            profile
                .validate_state(&decoded)
                .map_err(TransitionError::Domain)?;
            Some(decoded)
        }
        None => None,
    };
    profile
        .validate(current.as_ref(), &command)
        .map_err(TransitionError::Domain)?;
    let next = profile
        .apply(current.as_ref(), &command)
        .map_err(TransitionError::Domain)?;
    profile
        .validate_state(&next)
        .map_err(TransitionError::Domain)?;
    let capacity = output.len().min(P::MAX_PAYLOAD_BYTES);
    let bounded = output
        .get_mut(..capacity)
        .ok_or(TransitionError::Codec(CodecError::Capacity))?;
    let mut encoder = Encoder::new(bounded);
    next.encode(&mut encoder).map_err(TransitionError::Codec)?;
    let payload = encoder.written();
    let state_hash = custom_state_hash(P::DOMAIN_ID, P::SCHEMA_VERSION, payload)
        .map_err(TransitionError::Codec)?;
    Ok(TransitionResult {
        payload_len: payload.len(),
        version,
        state_hash,
    })
}

fn decode_canonical<T: CanonicalCodec>(
    bytes: &[u8],
    maximum: usize,
    scratch: &mut [u8],
) -> Result<T, CodecError> {
    if bytes.len() > maximum {
        return Err(CodecError::Capacity);
    }
    let mut reader = Decoder::new(bytes);
    let decoded = T::decode(&mut reader)?;
    reader.finish()?;
    let capacity = scratch.len().min(maximum);
    let mut writer = Encoder::new(scratch.get_mut(..capacity).ok_or(CodecError::Capacity)?);
    decoded.encode(&mut writer)?;
    if writer.written() != bytes {
        return Err(CodecError::NonCanonical);
    }
    Ok(decoded)
}
