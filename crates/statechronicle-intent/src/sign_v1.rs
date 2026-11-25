//! Typed cryptographic entry points for standalone v1 actor intent envelopes.

use statechronicle_core::{
    signature_operation::SignatureContext,
    signature_provider::{SignatureError, SignaturePurpose, Signer, Verifier},
    signature_wire::{decode_v1_envelope, encode_v1_envelope},
};
use statechronicle_domain::{
    intent::{INTENT_SCHEMA, Intent},
    signature_json::{decode_v1_json_envelope, encode_v1_json_envelope},
};

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

/// Converts a v1 intent JSON record to its canonical binary envelope.
///
/// The result still requires policy and signature verification. The
/// envelope signature authenticates the typed body independently of
/// JSON spelling and property order.
///
/// # Errors
/// Refuses invalid framing, duplicate fields, the wrong body schema, or an
/// unsupported value or size in the canonical body.
pub fn normalize_intent_json_v1(json: &[u8]) -> Result<Vec<u8>, IntentSignatureError> {
    let envelope = decode_v1_json_envelope(json)?;
    let mut intent: Intent = envelope.decode_body()?;
    if intent.schema != INTENT_V1_SCHEMA {
        return Err(SignatureError::MalformedEncoding.into());
    }
    intent.schema = String::from(INTENT_SCHEMA);
    let body = encode_v1_intent_body(&intent)?;
    Ok(encode_v1_envelope(
        envelope.with_canonical_body(&body),
        SignaturePurpose::ActorIntent,
    )?)
}

/// Renders a v1 binary intent as the typed JSON transport form.
///
/// # Errors
/// Refuses malformed envelopes, invalid canonical bodies, or JSON encoding failures.
pub fn render_intent_json_v1(record: &[u8]) -> Result<Vec<u8>, IntentSignatureError> {
    let envelope = decode_v1_envelope(record, SignaturePurpose::ActorIntent)?;
    let mut intent = decode_v1_intent_body(envelope.body)?;
    intent.schema = String::from(INTENT_V1_SCHEMA);
    Ok(encode_v1_json_envelope(&intent, envelope)?)
}
