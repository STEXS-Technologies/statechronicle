//! Ed25519 commit signing (protocol §18.1 step 14, ADR-004 §2, §5).
//!
//! Signs the BCS canonical bytes of a `Commit` body with the commit authority
//! key and wraps body + signature in the `Signed<Commit>` envelope. Per the
//! structural envelope rule (ADR-004 §2), the signature covers only the body,
//! never a `signature` field.

use ed25519_dalek::{SigningKey, VerifyingKey};

use statechronicle_core::canonicalize::canonicalize;
use statechronicle_core::signature::{sign, verify};

use statechronicle_domain::commit::Commit;
use statechronicle_domain::intent::{KeyId, SignatureAlg, SignatureBlock};
use statechronicle_domain::signed::Signed;

use crate::error::CommitError;

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
            CommitScope::tenant(TenantId(String::from("stexs.game.alpha"))),
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
            SubjectId(String::from("service:statechronicle.stexs.net")),
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
