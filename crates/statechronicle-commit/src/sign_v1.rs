//! Typed cryptographic entry points for v1 commits and checkpoints.
//!
//! The caller resolves authority policy before using these functions. Durable
//! admission and proof verification also require their existing domain checks.

use chrono::{DateTime, Utc};
use serde::{Serialize, de::DeserializeOwned};
use statechronicle_core::{
    canonicalize::canonicalize,
    signature_operation::SignatureContext,
    signature_provider::{SignatureError, SignaturePurpose, Signer, Verifier},
    signature_wire::{V1SignedEnvelope, decode_v1_envelope, encode_v1_envelope},
};
use statechronicle_domain::commit::{COMMIT_SCHEMA, Commit, ScopeKind};
use statechronicle_ports::{
    key_registry::KeyRegistryError,
    signature_policy::{
        ResolvedSignaturePolicy, SignatureAuthority, SignaturePolicy, SignaturePolicyQuery,
        SignatureUse,
    },
};

use crate::checkpoint::{GLOBAL_CHECKPOINT_SCHEMA, GlobalCheckpoint};

trait SigningBody: Serialize + DeserializeOwned {
    const PURPOSE: SignaturePurpose;
    const SCHEMA: &str;

    fn has_expected_schema(&self) -> bool;

    fn creation_time(&self) -> DateTime<Utc>;

    fn matches_authority(&self, authority: SignatureAuthority<'_>) -> bool;
}

impl SigningBody for Commit {
    const PURPOSE: SignaturePurpose = SignaturePurpose::TenantCommit;
    const SCHEMA: &str = COMMIT_SCHEMA;

    fn has_expected_schema(&self) -> bool {
        self.schema == Self::SCHEMA
            && self.scope.kind == ScopeKind::Tenant
            && self.scope.tenant_id.is_some()
    }

    fn creation_time(&self) -> DateTime<Utc> {
        self.created_at
    }

    fn matches_authority(&self, authority: SignatureAuthority<'_>) -> bool {
        matches!(authority, SignatureAuthority::TenantCommit { tenant, authority }
            if self.scope.tenant_id.as_ref() == Some(tenant) && self.executor == *authority)
    }
}

impl SigningBody for GlobalCheckpoint {
    const PURPOSE: SignaturePurpose = SignaturePurpose::GlobalCheckpoint;
    const SCHEMA: &str = GLOBAL_CHECKPOINT_SCHEMA;

    fn has_expected_schema(&self) -> bool {
        self.schema == Self::SCHEMA
    }

    fn creation_time(&self) -> DateTime<Utc> {
        self.created_at
    }

    fn matches_authority(&self, authority: SignatureAuthority<'_>) -> bool {
        matches!(authority, SignatureAuthority::GlobalCheckpoint { authority }
            if self.executor == *authority)
    }
}

fn check_context<T: SigningBody>(context: &SignatureContext<'_>) -> Result<(), SignatureError> {
    if context.purpose != T::PURPOSE || context.body_schema != T::SCHEMA {
        return Err(SignatureError::UnauthorizedSuite);
    }
    Ok(())
}

fn canonical_body<T: SigningBody>(body: &T) -> Result<Vec<u8>, SignatureError> {
    if !body.has_expected_schema() {
        return Err(SignatureError::MalformedEncoding);
    }
    canonicalize(body).map_err(|_encoding_error| SignatureError::MalformedEncoding)
}

fn sign_body<T: SigningBody>(
    body: &T,
    context: &SignatureContext<'_>,
    signer: &dyn Signer,
) -> Result<Vec<u8>, SignatureError> {
    check_context::<T>(context)?;
    let canonical = canonical_body(body)?;
    let signature = context.sign(&canonical, signer)?;
    encode_v1_envelope(
        V1SignedEnvelope {
            body: &canonical,
            suite_id: context.suite.as_str(),
            key_set_id: &context.key_set_id.0,
            signature: &signature,
        },
        T::PURPOSE,
    )
}

fn verify_body<T: SigningBody>(
    record: &[u8],
    context: &SignatureContext<'_>,
    verifier: &dyn Verifier,
) -> Result<T, SignatureError> {
    check_context::<T>(context)?;
    let envelope = decode_v1_envelope(record, T::PURPOSE)?;
    let body = decode_body::<T>(envelope)?;
    verify_signature(envelope, context, verifier)?;
    Ok(body)
}

fn decode_body<T: SigningBody>(envelope: V1SignedEnvelope<'_>) -> Result<T, SignatureError> {
    let body = bcs::from_bytes(envelope.body)
        .map_err(|_decoding_error| SignatureError::MalformedEncoding)?;
    if canonical_body(&body)? != envelope.body {
        return Err(SignatureError::MalformedEncoding);
    }
    Ok(body)
}

fn verify_signature(
    envelope: V1SignedEnvelope<'_>,
    context: &SignatureContext<'_>,
    verifier: &dyn Verifier,
) -> Result<(), SignatureError> {
    if context.suite.as_str() != envelope.suite_id || context.key_set_id.0 != *envelope.key_set_id {
        return Err(SignatureError::UnauthorizedSuite);
    }
    context.verify(
        envelope.body,
        context.suite,
        context.key_set_id,
        envelope.signature,
        verifier,
    )
}

/// Failure to resolve or verify a v1 authority signature.
#[derive(Debug, thiserror::Error)]
pub enum PolicySignatureError {
    /// The key registry refused or could not resolve the request.
    #[error(transparent)]
    Registry(#[from] KeyRegistryError),
    /// The record exceeded bounds or failed policy or signature validation.
    #[error(transparent)]
    Signature(#[from] SignatureError),
}

async fn verify_with_policy<T: SigningBody>(
    record: &[u8],
    authority: SignatureAuthority<'_>,
    usage: SignatureUse,
    now: DateTime<Utc>,
    policy: &dyn SignaturePolicy,
) -> Result<T, PolicySignatureError> {
    if authority.purpose() != T::PURPOSE {
        return Err(SignatureError::UnauthorizedSuite.into());
    }
    let envelope = decode_v1_envelope(record, T::PURPOSE)?;
    let body = decode_body::<T>(envelope)?;
    if !body.matches_authority(authority) || body.creation_time() > now {
        return Err(SignatureError::UnauthorizedSuite.into());
    }
    let decision = policy
        .resolve(SignaturePolicyQuery {
            authority,
            usage,
            signed_at: body.creation_time(),
            now,
        })
        .await?;
    let ResolvedSignaturePolicy::V1(resolved) = decision else {
        return Err(SignatureError::UnauthorizedSuite.into());
    };
    verify_signature(
        envelope,
        &resolved.context(T::PURPOSE, T::SCHEMA),
        resolved.verifier(),
    )?;
    Ok(body)
}

/// Verifies a v1 tenant commit against the independently configured authority.
///
/// # Errors
/// Returns policy, decoding, or cryptographic errors. Historical mode requires
/// the caller to have independently pinned the signed record.
pub async fn verify_commit_v1_with_policy(
    record: &[u8],
    authority: SignatureAuthority<'_>,
    usage: SignatureUse,
    now: DateTime<Utc>,
    policy: &dyn SignaturePolicy,
) -> Result<Commit, PolicySignatureError> {
    verify_with_policy(record, authority, usage, now, policy).await
}

/// Verifies a v1 global checkpoint against the independently configured authority.
///
/// # Errors
/// Returns policy, decoding, or cryptographic errors. Historical mode requires
/// the caller to have independently pinned the signed record.
pub async fn verify_global_checkpoint_v1_with_policy(
    record: &[u8],
    authority: SignatureAuthority<'_>,
    usage: SignatureUse,
    now: DateTime<Utc>,
    policy: &dyn SignaturePolicy,
) -> Result<GlobalCheckpoint, PolicySignatureError> {
    verify_with_policy(record, authority, usage, now, policy).await
}

/// Signs a tenant commit and returns its standalone v1 BCS envelope.
///
/// # Errors
/// Rejects a mismatched scope, schema, or context, and propagates provider errors.
pub fn sign_commit_v1(
    body: &Commit,
    context: &SignatureContext<'_>,
    signer: &dyn Signer,
) -> Result<Vec<u8>, SignatureError> {
    sign_body(body, context, signer)
}

/// Verifies and decodes a standalone v1 tenant commit envelope.
///
/// # Errors
/// Rejects malformed or noncanonical records, context mismatches, and invalid signatures.
pub fn verify_commit_v1(
    record: &[u8],
    context: &SignatureContext<'_>,
    verifier: &dyn Verifier,
) -> Result<Commit, SignatureError> {
    verify_body(record, context, verifier)
}

/// Signs a global checkpoint and returns its standalone v1 BCS envelope.
///
/// # Errors
/// Rejects a mismatched schema or context, and propagates provider errors.
pub fn sign_global_checkpoint_v1(
    body: &GlobalCheckpoint,
    context: &SignatureContext<'_>,
    signer: &dyn Signer,
) -> Result<Vec<u8>, SignatureError> {
    sign_body(body, context, signer)
}

/// Verifies and decodes a standalone v1 global checkpoint envelope.
///
/// # Errors
/// Rejects malformed or noncanonical records, context mismatches, and invalid signatures.
pub fn verify_global_checkpoint_v1(
    record: &[u8],
    context: &SignatureContext<'_>,
    verifier: &dyn Verifier,
) -> Result<GlobalCheckpoint, SignatureError> {
    verify_body(record, context, verifier)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use ed25519_dalek::SigningKey;
    use statechronicle_core::signature_provider::{Ed25519Signer, Ed25519Verifier, KeySetId};
    use statechronicle_domain::signed::Signed;

    use super::*;

    struct CheckpointPolicy(Option<std::sync::Arc<dyn Verifier>>);

    #[async_trait::async_trait]
    impl SignaturePolicy for CheckpointPolicy {
        async fn resolve(
            &self,
            query: SignaturePolicyQuery<'_>,
        ) -> Result<ResolvedSignaturePolicy, KeyRegistryError> {
            if query.authority.purpose() != SignaturePurpose::GlobalCheckpoint
                || query.usage != SignatureUse::Active
            {
                return Err(KeyRegistryError::NotTrusted);
            }
            Ok(self
                .0
                .as_ref()
                .map_or(ResolvedSignaturePolicy::Legacy, |verifier| {
                    ResolvedSignaturePolicy::V1(
                        statechronicle_ports::signature_policy::ResolvedSignatureV1::new(
                            verifier.clone(),
                            KeySetId([1; 32]),
                            64,
                        ),
                    )
                }))
        }
    }

    #[tokio::test]
    async fn v1_checkpoint_verifies_under_its_own_purpose() {
        let legacy: Signed<GlobalCheckpoint> = serde_json::from_slice(include_bytes!(
            "../tests/fixtures/wire-v0.3.0/signed-global-checkpoint.json"
        ))
        .unwrap();
        let signing_key = SigningKey::from_bytes(&[42; 32]);
        let verifying_key = signing_key.verifying_key();
        let key_set = KeySetId([1; 32]);
        let purpose = SignaturePurpose::GlobalCheckpoint;
        let signer = Ed25519Signer::new(&signing_key, key_set, purpose);
        let verifier = Ed25519Verifier::new(&verifying_key, key_set, purpose);
        let context = SignatureContext {
            suite: signer.suite(),
            key_set_id: &key_set,
            purpose,
            body_schema: GLOBAL_CHECKPOINT_SCHEMA,
            signature_len: 64,
        };

        let record = sign_global_checkpoint_v1(&legacy.body, &context, &signer).unwrap();
        assert_eq!(
            verify_global_checkpoint_v1(&record, &context, &verifier).unwrap(),
            legacy.body
        );
        let authority = SignatureAuthority::GlobalCheckpoint {
            authority: &legacy.body.executor,
        };
        let policy = CheckpointPolicy(Some(std::sync::Arc::new(Ed25519Verifier::new(
            &verifying_key,
            key_set,
            purpose,
        ))));
        assert_eq!(
            verify_global_checkpoint_v1_with_policy(
                &record,
                authority,
                SignatureUse::Active,
                legacy.body.created_at,
                &policy,
            )
            .await
            .unwrap(),
            legacy.body
        );
        assert!(matches!(
            verify_global_checkpoint_v1_with_policy(
                &record,
                authority,
                SignatureUse::Active,
                legacy.body.created_at,
                &CheckpointPolicy(None),
            )
            .await,
            Err(PolicySignatureError::Signature(
                SignatureError::UnauthorizedSuite
            ))
        ));
        assert_eq!(
            verify_commit_v1(&record, &context, &verifier),
            Err(SignatureError::UnauthorizedSuite)
        );
        assert!(
            verify_global_checkpoint_v1(
                include_bytes!("../tests/fixtures/wire-v0.3.0/signed-global-checkpoint.bcs"),
                &context,
                &verifier,
            )
            .is_err()
        );
    }
}
