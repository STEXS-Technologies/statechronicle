//! Typed cryptographic entry points for v1 commits and checkpoints.
//!
//! The caller resolves authority policy before using these functions. Durable
//! admission and proof verification also require their existing domain checks.

use chrono::{DateTime, Utc};
use serde::{Serialize, de::DeserializeOwned};
use statechronicle_core::{
    canonicalize::canonicalize,
    signature_operation::SignatureContext,
    signature_provider::{AsyncSigner, SignatureError, SignaturePurpose, Signer, Verifier},
    signature_wire::{V1SignedEnvelope, decode_v1_envelope, encode_v1_envelope},
};
use statechronicle_domain::{
    commit::{COMMIT_SCHEMA, Commit, ScopeKind},
    signature_json::{decode_v1_json_envelope, encode_v1_json_envelope},
};
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
    context.sign_envelope(&canonical, signer)
}

async fn sign_body_async<T: SigningBody>(
    body: &T,
    context: &SignatureContext<'_>,
    signer: &dyn AsyncSigner,
) -> Result<Vec<u8>, SignatureError> {
    check_context::<T>(context)?;
    let canonical = canonical_body(body)?;
    context.sign_envelope_async(&canonical, signer).await
}

fn verify_body<T: SigningBody>(
    record: &[u8],
    context: &SignatureContext<'_>,
    verifier: &dyn Verifier,
) -> Result<T, SignatureError> {
    check_context::<T>(context)?;
    let envelope = decode_v1_envelope(record, T::PURPOSE)?;
    let body = decode_body::<T>(envelope)?;
    context.verify_envelope(envelope, verifier)?;
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
    resolved
        .context(T::PURPOSE, T::SCHEMA)
        .verify_envelope(envelope, resolved.verifier())?;
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

/// Signs a v1 tenant commit through a nonblocking provider.
///
/// # Errors
/// Rejects mismatched scope, context, or provider output and propagates provider failure.
pub async fn sign_commit_v1_async(
    body: &Commit,
    context: &SignatureContext<'_>,
    signer: &dyn AsyncSigner,
) -> Result<Vec<u8>, SignatureError> {
    sign_body_async(body, context, signer).await
}

/// Decodes a v1 tenant commit and validates its canonical body encoding.
///
/// The result still requires signature and policy verification.
///
/// # Errors
/// Rejects malformed envelope bytes, schema or scope, and noncanonical bodies.
pub fn decode_commit_v1(record: &[u8]) -> Result<Commit, SignatureError> {
    decode_body(decode_v1_envelope(record, SignaturePurpose::TenantCommit)?)
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

/// Signs a v1 global checkpoint through a nonblocking provider.
///
/// # Errors
/// Rejects mismatched schema, context, or provider output and propagates provider failure.
pub async fn sign_global_checkpoint_v1_async(
    body: &GlobalCheckpoint,
    context: &SignatureContext<'_>,
    signer: &dyn AsyncSigner,
) -> Result<Vec<u8>, SignatureError> {
    sign_body_async(body, context, signer).await
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

fn normalize_body_json<T: SigningBody>(json: &[u8]) -> Result<Vec<u8>, SignatureError> {
    let envelope = decode_v1_json_envelope(json)?;
    let body: T = envelope.decode_body()?;
    let canonical = canonical_body(&body)?;
    encode_v1_envelope(envelope.with_canonical_body(&canonical), T::PURPOSE)
}

fn render_body_json<T: SigningBody>(record: &[u8]) -> Result<Vec<u8>, SignatureError> {
    let envelope = decode_v1_envelope(record, T::PURPOSE)?;
    let body = decode_body::<T>(envelope)?;
    encode_v1_json_envelope(&body, envelope)
}

/// Converts a v1 tenant commit JSON envelope to a canonical binary envelope.
///
/// The result requires policy and signature verification before use.
///
/// # Errors
/// Rejects malformed framing, duplicate fields, and invalid body schema or scope.
pub fn normalize_commit_json_v1(json: &[u8]) -> Result<Vec<u8>, SignatureError> {
    normalize_body_json::<Commit>(json)
}

/// Converts a v1 global checkpoint JSON envelope to a canonical binary envelope.
///
/// The result requires policy and signature verification before use.
///
/// # Errors
/// Rejects malformed framing, duplicate fields, and invalid body schema.
pub fn normalize_checkpoint_json_v1(json: &[u8]) -> Result<Vec<u8>, SignatureError> {
    normalize_body_json::<GlobalCheckpoint>(json)
}

/// Renders a canonical v1 tenant commit envelope as typed JSON.
///
/// # Errors
/// Rejects malformed envelope or body bytes, and JSON encoding failures.
pub fn render_commit_json_v1(record: &[u8]) -> Result<Vec<u8>, SignatureError> {
    render_body_json::<Commit>(record)
}

/// Renders a canonical v1 global checkpoint envelope as typed JSON.
///
/// # Errors
/// Rejects malformed envelope or body bytes, and JSON encoding failures.
pub fn render_checkpoint_json_v1(record: &[u8]) -> Result<Vec<u8>, SignatureError> {
    render_body_json::<GlobalCheckpoint>(record)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use ed25519_dalek::SigningKey;
    use statechronicle_core::signature_provider::{Ed25519Signer, Ed25519Verifier, KeySetId};
    use statechronicle_domain::signed::Signed;

    use super::*;

    struct DeferredSigner<'signer> {
        local: &'signer dyn Signer,
        result: Option<Result<usize, SignatureError>>,
    }

    impl AsyncSigner for DeferredSigner<'_> {
        fn suite(&self) -> &statechronicle_core::signature_provider::SignatureSuiteId {
            self.local.suite()
        }

        fn sign_into<'operation>(
            &'operation self,
            request: statechronicle_core::signature_provider::SigningRequest<'operation>,
            output: &'operation mut [u8],
        ) -> statechronicle_core::signature_provider::SigningFuture<'operation> {
            Box::pin(async move {
                tokio::task::yield_now().await;
                let written = self.local.sign_into(request, output)?;
                self.result.unwrap_or(Ok(written))
            })
        }
    }

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
                        )
                        .unwrap(),
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
        let remote = DeferredSigner {
            local: &signer,
            result: None,
        };
        assert_eq!(
            sign_global_checkpoint_v1_async(&legacy.body, &context, &remote)
                .await
                .unwrap(),
            record
        );
        for (provider_result, expected_error) in [
            (Ok(1), SignatureError::MalformedEncoding),
            (
                Err(SignatureError::ProviderUnavailable),
                SignatureError::ProviderUnavailable,
            ),
        ] {
            let failing_remote = DeferredSigner {
                local: &signer,
                result: Some(provider_result),
            };
            assert_eq!(
                sign_global_checkpoint_v1_async(&legacy.body, &context, &failing_remote).await,
                Err(expected_error)
            );
        }
        let json = render_checkpoint_json_v1(&record).unwrap();
        assert_eq!(normalize_checkpoint_json_v1(&json).unwrap(), record);
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
