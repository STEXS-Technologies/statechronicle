//! Typed cryptographic entry points for standalone v1 actor intent envelopes.

use statechronicle_core::{
    signature_operation::SignatureContext,
    signature_provider::{SignatureError, SignaturePurpose, Signer, Verifier},
    signature_wire::decode_v1_envelope,
};
use statechronicle_domain::intent::Intent;

use crate::{
    error::IntentError,
    v1_body::{INTENT_V1_SCHEMA, decode_v1_intent_body, encode_v1_intent_body},
};

/// A v1 actor intent failed codec or signature checks.
#[derive(Debug, thiserror::Error)]
pub enum IntentSignatureError {
    /// The typed intent body or its encoding is invalid.
    #[error(transparent)]
    Body(#[from] IntentError),
    /// Envelope or signature verification failed.
    #[error(transparent)]
    Signature(#[from] SignatureError),
}

fn check_context(context: &SignatureContext<'_>) -> Result<(), SignatureError> {
    if context.purpose != SignaturePurpose::ActorIntent || context.body_schema != INTENT_V1_SCHEMA {
        return Err(SignatureError::UnauthorizedSuite);
    }
    Ok(())
}

/// Signs a typed actor intent with the selected v1 provider.
///
/// # Errors
/// Rejects incorrect context, noncanonical body inputs, and provider failures.
pub fn sign_intent_v1(
    intent: &Intent,
    context: &SignatureContext<'_>,
    signer: &dyn Signer,
) -> Result<Vec<u8>, IntentSignatureError> {
    check_context(context)?;
    let body = encode_v1_intent_body(intent)?;
    Ok(context.sign_envelope(&body, signer)?)
}

/// Verifies and decodes a v1 actor intent under an independently resolved context.
///
/// Callers retain the original record and run identity, expiry, authorization,
/// and replay checks before executing the decoded intent.
///
/// # Errors
/// Rejects malformed envelopes or bodies, context mismatches, and invalid signatures.
pub fn verify_intent_v1(
    record: &[u8],
    context: &SignatureContext<'_>,
    verifier: &dyn Verifier,
) -> Result<Intent, IntentSignatureError> {
    check_context(context)?;
    let envelope = decode_v1_envelope(record, SignaturePurpose::ActorIntent)?;
    let body = decode_v1_intent_body(envelope.body)?;
    context.verify_envelope(envelope, verifier)?;
    Ok(body)
}
