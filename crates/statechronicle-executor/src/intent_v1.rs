//! Policy-bound authentication of an actor's v1 intent before preparation.

use chrono::{DateTime, Utc};
use statechronicle_core::{
    digest::{ContentDigest, hash_bytes},
    signature_provider::SignaturePurpose,
    signature_wire::decode_v1_envelope,
};
use statechronicle_intent::{
    v1_body::{INTENT_V1_SCHEMA, decode_v1_intent_body},
    validated::ValidatedIntent,
};
use statechronicle_ports::{
    authorization::{AuthenticatedPrincipal, Authorizer},
    signature_policy::{
        ResolvedSignaturePolicy, SignatureAuthority, SignaturePolicy, SignaturePolicyQuery,
        SignatureUse,
    },
};

use crate::{
    conflict::check_expiry,
    error::ExecutorError,
    pipeline::{authorize_principal, check_principal_binding},
};

/// An authorized v1 intent and its exact signed evidence.
///
/// Preparation and durable admission still enforce their own state and replay
/// invariants. The legacy typed view is used for application semantics.
#[derive(Debug)]
pub struct AuthenticatedIntentV1<'record> {
    validated: ValidatedIntent,
    record: &'record [u8],
    body_digest: ContentDigest,
}

impl<'record> AuthenticatedIntentV1<'record> {
    /// The structurally checked intent and idempotency tuple.
    pub const fn validated(&self) -> &ValidatedIntent {
        &self.validated
    }

    /// The original canonical signed envelope.
    pub const fn record(&self) -> &'record [u8] {
        self.record
    }

    /// Digest of the versioned canonical body for exact replay identity.
    pub const fn body_digest(&self) -> &ContentDigest {
        &self.body_digest
    }
}

/// Authenticates a standalone v1 intent against current policy and authorization.
///
/// `now` comes from the deployment's trusted clock. Policy selection uses
/// the authenticated actor and tenant, and the operation later bound by the
/// signature. No state mutations or replay claims occur during authentication.
///
/// # Errors
/// Rejects invalid bodies, scope or time mismatches, inactive policy,
/// invalid signatures, and operations the authorizer refuses.
pub async fn authenticate_player_intent_v1<'record>(
    record: &'record [u8],
    principal: &AuthenticatedPrincipal,
    authorizer: &dyn Authorizer,
    policy: &dyn SignaturePolicy,
    now: DateTime<Utc>,
) -> Result<AuthenticatedIntentV1<'record>, ExecutorError> {
    let purpose = SignaturePurpose::ActorIntent;
    let envelope = decode_v1_envelope(record, purpose)
        .map_err(|error| ExecutorError::ActorAuthenticationFailed(error.to_string()))?;
    let intent = decode_v1_intent_body(envelope.body)?;
    check_principal_binding(&intent, principal)?;
    check_expiry(&intent, now)?;
    if intent.created_at > now {
        return Err(ExecutorError::ActorAuthenticationFailed(String::from(
            "intent timestamp is ahead of the trusted clock",
        )));
    }
    let validated = ValidatedIntent::try_from_intent(intent, None)?;
    let resolved = policy
        .resolve(SignaturePolicyQuery {
            authority: SignatureAuthority::ActorIntent {
                tenant: &principal.tenant,
                actor: &principal.subject,
                operation: &validated.intent.operation,
            },
            usage: SignatureUse::Active,
            signed_at: validated.intent.created_at,
            now,
        })
        .await
        .map_err(|error| ExecutorError::ActorAuthenticationFailed(error.to_string()))?;
    let ResolvedSignaturePolicy::V1(resolved) = resolved else {
        return Err(ExecutorError::ActorAuthenticationFailed(String::from(
            "trusted policy requires a legacy intent signature",
        )));
    };
    resolved
        .context(purpose, INTENT_V1_SCHEMA)
        .verify_envelope(envelope, resolved.verifier())
        .map_err(|error| ExecutorError::ActorAuthenticationFailed(error.to_string()))?;
    authorize_principal(&validated, principal, authorizer).await?;

    Ok(AuthenticatedIntentV1 {
        validated,
        record,
        body_digest: hash_bytes(envelope.body),
    })
}
