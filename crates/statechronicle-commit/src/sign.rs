//! Ed25519 commit signing (protocol §18.1 step 14, ADR-004 §2, §5).
//!
//! Signs the BCS canonical bytes of a `Commit` body with the commit authority
//! key and wraps body + signature in the `Signed<Commit>` envelope. Per the
//! structural envelope rule (ADR-004 §2), the signature covers only the body,
//! never a `signature` field.

use ed25519_dalek::{SigningKey, VerifyingKey};

use statechronicle_core::canonicalize::canonicalize;
use statechronicle_core::signature::Signature;
use statechronicle_core::signature::{sign, verify};

use statechronicle_domain::commit::Commit;
use statechronicle_domain::intent::{KeyId, SignatureAlg, SignatureBlock};
use statechronicle_domain::signed::Signed;

use crate::error::CommitError;
use crate::persist::SignedCommitVerifier;

/// Deployment-provided commit signer boundary.
///
/// Production implementations should delegate `sign` to a KMS/HSM and
/// return only the detached signature. The private key never needs to enter
/// the StateChronicle process. The `key_id` is supplied so the provider can
/// enforce tenant/scope rotation policy before signing.
pub trait CommitSigner: Send + Sync {
    /// Signs canonical commit-body bytes for `key_id`.
    ///
    /// # Errors
    ///
    /// Returns [`CommitError`] when canonicalization fails or the provider
    /// rejects/unavailable the signing request.
    fn sign_bytes(&self, key_id: &KeyId, canonical: &[u8]) -> Result<Signature, CommitError>;
}

/// Forms a signed commit through a deployment-provided signer.
///
/// Unlike [`sign_commit`], this function does not require a local private key
/// and is suitable for a remote KMS/HSM adapter.
///
/// # Errors
///
/// Returns [`CommitError::Core`] when canonicalization fails, or the provider
/// error returned by [`CommitSigner::sign_bytes`].
pub fn sign_commit_with_signer(
    body: &Commit,
    key_id: KeyId,
    signer: &dyn CommitSigner,
) -> Result<Signed<Commit>, CommitError> {
    let canonical = canonicalize(body)?;
    let signature = signer.sign_bytes(&key_id, &canonical)?;
    Ok(Signed::new(
        body.clone(),
        SignatureBlock {
            alg: SignatureAlg::Ed25519,
            key_id,
            sig: signature,
        },
    ))
}

/// Adapter that resolves a commit key through a caller-supplied registry and
/// then performs strict Ed25519 verification. The resolver must enforce tenant,
/// validity, and revocation policy before returning a verifying key.
pub struct Ed25519CommitVerifier<F> {
    resolver: F,
}

impl<F> Ed25519CommitVerifier<F> {
    /// Creates a verifier from a key-resolution callback.
    pub const fn new(resolver: F) -> Self {
        Self { resolver }
    }
}

impl<F> SignedCommitVerifier for Ed25519CommitVerifier<F>
where
    F: Fn(&Signed<Commit>) -> Result<VerifyingKey, String> + Send + Sync,
{
    fn verify(&self, commit: &Signed<Commit>) -> Result<(), String> {
        let key = (self.resolver)(commit)?;
        verify_commit(commit, &key).map_err(|error| error.to_string())
    }
}

/// Signs a commit body and wraps it in the signed envelope.
///
/// # Errors
///
/// Returns [`CommitError::Core`] when the body cannot be BCS canonicalized.
pub fn sign_commit(
    body: &Commit,
    key: &SigningKey,
    key_id: KeyId,
) -> Result<Signed<Commit>, CommitError> {
    let canonical = canonicalize(body)?;
    let signature = sign(&canonical, key);
    let block = SignatureBlock {
        alg: SignatureAlg::Ed25519,
        key_id,
        sig: signature,
    };
    Ok(Signed::new(body.clone(), block))
}

/// Verifies a signed commit's detached signature over the BCS body bytes.
///
/// # Errors
///
/// Returns [`CommitError::Core`] when the body cannot be BCS canonicalized or
/// the signature fails strict Ed25519 verification (ZIP-215 malleability
/// checks).
pub fn verify_commit(
    signed: &Signed<Commit>,
    verifying_key: &VerifyingKey,
) -> Result<(), CommitError> {
    if signed.signature.alg != SignatureAlg::Ed25519 {
        return Err(CommitError::Core(
            statechronicle_core::error::StateChronicleError::SignatureVerification(String::from(
                "unsupported commit signature algorithm",
            )),
        ));
    }
    let canonical = canonicalize(&signed.body)?;
    verify(&canonical, verifying_key, &signed.signature.sig)?;
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;
    use chrono::{DateTime, Utc};
    use ed25519_dalek::SigningKey;
    use statechronicle_core::digest::hash_bytes;
    use statechronicle_domain::commit::{CommitScope, ProfileId};
    use statechronicle_domain::ids::CommitId;
    use statechronicle_domain::subject::SubjectId;
    use statechronicle_domain::tenant::TenantId;

    const FIXED_SEED: [u8; 32] = [42u8; 32];

    fn fixed_key() -> SigningKey {
        SigningKey::from_bytes(&FIXED_SEED)
    }

    fn key_id() -> KeyId {
        KeyId::new(String::from("did:key:z6Mk...#statechronicle-commit")).unwrap()
    }

    fn sample_commit() -> Commit {
        Commit::new(
            CommitScope::tenant(TenantId(String::from("acme.game.alpha"))),
            CommitId::new(String::from("cmt_01JZ8X5HN3C4PXG5A9FGEWQF5W")).unwrap(),
            None,
            1,
            2,
            hash_bytes(b"event-root"),
            hash_bytes(b"previous-root"),
            hash_bytes(b"next-root"),
            DateTime::parse_from_rfc3339("2026-07-14T00:00:02Z")
                .unwrap()
                .with_timezone(&Utc),
            SubjectId(String::from("service:statechronicle.example.net")),
            ProfileId::new(String::from("statechronicle.profile.resource.v0")).unwrap(),
        )
    }

    #[test]
    fn sign_then_verify_succeeds() {
        let body = sample_commit();
        let key = fixed_key();
        let signed = sign_commit(&body, &key, key_id()).unwrap();
        assert_eq!(signed.body, body);
        assert_eq!(signed.signature.alg, SignatureAlg::Ed25519);
        assert_eq!(
            signed.signature.key_id.as_str(),
            "did:key:z6Mk...#statechronicle-commit"
        );
        assert!(verify_commit(&signed, &key.verifying_key()).is_ok());
        let verifier = Ed25519CommitVerifier::new(move |_commit: &Signed<Commit>| {
            Ok::<VerifyingKey, String>(key.verifying_key())
        });
        assert!(verifier.verify(&signed).is_ok());
    }

    struct TestSigner(SigningKey);

    impl CommitSigner for TestSigner {
        fn sign_bytes(&self, _key_id: &KeyId, canonical: &[u8]) -> Result<Signature, CommitError> {
            Ok(sign(canonical, &self.0))
        }
    }

    #[test]
    fn signer_provider_forms_verifiable_commit_without_local_api() {
        let body = sample_commit();
        let key = fixed_key();
        let signed = sign_commit_with_signer(&body, key_id(), &TestSigner(key.clone())).unwrap();
        assert!(verify_commit(&signed, &key.verifying_key()).is_ok());
    }

    #[test]
    fn verify_rejects_wrong_key() {
        let body = sample_commit();
        let key = fixed_key();
        let signed = sign_commit(&body, &key, key_id()).unwrap();
        let other = SigningKey::from_bytes(&[7u8; 32]);
        assert!(matches!(
            verify_commit(&signed, &other.verifying_key()),
            Err(CommitError::Core(_))
        ));
    }

    #[test]
    fn verify_rejects_tampered_body() {
        let body = sample_commit();
        let key = fixed_key();
        let mut signed = sign_commit(&body, &key, key_id()).unwrap();
        signed.body.sequence = signed.body.sequence.wrapping_add(1);
        assert!(matches!(
            verify_commit(&signed, &key.verifying_key()),
            Err(CommitError::Core(_))
        ));
    }

    #[test]
    fn signature_is_deterministic_for_fixed_key_and_body() {
        let body = sample_commit();
        let key = fixed_key();
        let first = sign_commit(&body, &key, key_id()).unwrap();
        let second = sign_commit(&body, &key, key_id()).unwrap();
        assert_eq!(
            first.signature.sig.as_bytes(),
            second.signature.sig.as_bytes()
        );
    }
}
