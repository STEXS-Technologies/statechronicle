//! Domain-aware verification of consumer-defined state proofs.
use crate::claim::VerifiedClaimV1;
use crate::error::ProofError;
use crate::verify::verify_bundle;
use ed25519_dalek::VerifyingKey;
use serde::{Serialize, de::DeserializeOwned};
use statechronicle_accumulator::key::StateKey;
use statechronicle_domain::{
    commit::Commit, proof::ResourceStateProof, resource_state::ResourceState, signed::Signed,
};
use statechronicle_profiles::{StateProfile, custom::TypedProfile};
use std::fmt::Display;

/// Authenticates inclusion under a signed commit, then checks domain identity,
/// schema version, canonical payload encoding and consumer state invariants.
/// Commit signer trust and canonical-head freshness remain the caller's policy.
/// # Errors
/// Returns proof verification failures or InvalidState for domain/schema/payload errors.
pub fn verify_custom_bundle<P>(
    proof: &ResourceStateProof,
    signed: &Signed<Commit>,
    verifying_key: &VerifyingKey,
    key: &StateKey,
    profile: &TypedProfile<P>,
) -> Result<P::State, ProofError>
where
    P: StateProfile,
    P::State: Serialize + DeserializeOwned,
    P::Command: Serialize + DeserializeOwned,
    P::Error: Display,
{
    verify_bundle(proof, signed, verifying_key, key)?;
    let claimed_key = StateKey::for_resource(&proof.tenant_id.0, &proof.resource_id.0);
    if claimed_key != *key {
        return Err(ProofError::InvalidState(String::from(
            "custom proof resource/key mismatch",
        )));
    }
    decode_custom_state(&proof.claimed_state, profile)
}

/// Decodes a verified v1 resource claim through its expected consumer profile.
///
/// # Errors
/// Refuses absence, a subject-keyed address, or invalid custom domain,
/// schema, canonical encoding, or state invariants.
pub fn decode_custom_claim_v1<P>(
    claim: &VerifiedClaimV1,
    profile: &TypedProfile<P>,
) -> Result<P::State, ProofError>
where
    P: StateProfile,
    P::State: Serialize + DeserializeOwned,
    P::Command: Serialize + DeserializeOwned,
    P::Error: Display,
{
    if claim.expectation().subject().is_some() {
        return Err(ProofError::InvalidState(String::from(
            "custom proof resource/key mismatch",
        )));
    }
    let state = claim.state().ok_or_else(|| {
        ProofError::InvalidState(String::from("custom claim requires present state"))
    })?;
    decode_custom_state(state, profile)
}

fn decode_custom_state<P>(
    state: &ResourceState,
    profile: &TypedProfile<P>,
) -> Result<P::State, ProofError>
where
    P: StateProfile,
    P::State: Serialize + DeserializeOwned,
    P::Command: Serialize + DeserializeOwned,
    P::Error: Display,
{
    profile
        .decode_state(state)
        .map_err(|error| ProofError::InvalidState(error.to_string()))
}
