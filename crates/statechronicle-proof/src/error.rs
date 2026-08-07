//! Proof assembly and verification error type.
//!
//! Failures from bundle construction and verification, built with `thiserror`.
//! Every fallible public function in `statechronicle-proof` returns this type.
//! Variants are typed and carry structured context so callers can fail closed
//! without string matching (CODE_STANDARDS §5).

use statechronicle_core::error::StateChronicleError;
use statechronicle_domain::error::DomainError;

/// The crate's root error type.
///
/// All fallible proof operations return this type. Errors distinguish
/// structural failures (unsupported schema/kind, malformed paths, missing
/// subject) from cryptographic failures (signature, inclusion, claimed-state
/// mismatch) and backend failures (store, proof index), so verifiers and
/// services can map each to the right status without string matching.
#[derive(Debug, thiserror::Error)]
pub enum ProofError {
    /// A domain type construction failed.
    ///
    /// Raised when a `statechronicle-domain` newtype (commit id, event id,
    /// key id, ...) rejects a value.
    #[error(transparent)]
    Domain(#[from] DomainError),

    /// A core primitive (canonicalization, digest, or signature) failed.
    ///
    /// Raised when BCS canonical serialization or Ed25519 strict
    /// verification fails (ADR-004 §4–5).
    #[error(transparent)]
    Core(StateChronicleError),

    /// No proof could be produced for the requested claim.
    #[error("no proof available for the requested claim")]
    NotFound,

    /// The proof bundle uses an unsupported schema identifier.
    #[error("unsupported proof schema `{0}`")]
    UnsupportedSchema(String),

    /// The state inclusion proof uses an unsupported encoding kind.
    #[error("unsupported inclusion proof kind `{0}`")]
    UnsupportedKind(String),

    /// A proof's claimed state is structurally invalid.
    ///
    /// Raised when a required field (for example `owner` or `subject`) is
    /// missing or not a string in the claimed state payload.
    #[error("claimed state is invalid: {0}")]
    InvalidState(String),

    /// Ed25519 commit signature verification failed.
    ///
    /// Raised by [`crate::verify::verify_commit_signature_with_key`] when the
    /// detached commit signature does not verify over the BCS canonical
    /// commit body under the supplied key (protocol §16.3 step 2).
    #[error("commit signature verification failed: {0}")]
    CommitSignature(String),

    /// The proof's embedded commit reference does not match the signed commit
    /// it claims to pin.
    #[error("commit reference mismatch: expected `{expected}`, got `{actual}`")]
    CommitRefMismatch {
        /// The expected reference (from the enclosing signed commit).
        expected: String,
        /// The reference carried by the proof.
        actual: String,
    },

    /// The enclosing commit is not tenant-scoped.
    #[error("commit is not tenant-scoped: {0}")]
    CommitScope(String),

    /// A verifying key could not be resolved for the referenced key id.
    #[error("verifying key not found for key id `{0}`")]
    KeyNotFound(String),

    /// The proof's tenant does not match the expected tenant.
    #[error("proof tenant `{actual}` does not match expected tenant `{expected}`")]
    TenantMismatch {
        /// The expected tenant id string.
        expected: String,
        /// The tenant id carried by the proof.
        actual: String,
    },

    /// The proof's resource does not match the expected resource.
    #[error("proof resource `{actual}` does not match expected resource `{expected}`")]
    ResourceMismatch {
        /// The expected resource id string.
        expected: String,
        /// The resource id carried by the proof.
        actual: String,
    },

    /// The proof's owner does not match the expected subject.
    #[error("proof owner `{actual}` does not match expected subject `{expected}`")]
    SubjectMismatch {
        /// The expected subject id string.
        expected: String,
        /// The owner carried by the claimed state.
        actual: String,
    },

    /// The claimed state carries no `owner` field.
    #[error("claimed state is missing an owner field")]
    MissingOwner,

    /// The sparse Merkle inclusion proof does not verify against the root.
    #[error("state inclusion proof does not verify against the commit state root")]
    InclusionMismatch,

    /// The claimed state does not hash to the included leaf.
    #[error("claimed state does not match the included leaf")]
    ClaimedStateMismatch,

    /// The inclusion proof's leaf does not commit the projected state hash.
    #[error("inclusion proof leaf does not commit the projected state hash")]
    LeafMismatch,

    /// A dense v0 sparse Merkle path has the wrong length.
    #[error("sparse merkle proof path has invalid length: expected {expected}, got {actual}")]
    InvalidPathLength {
        /// The required dense path length (256).
        expected: usize,
        /// The actual path length.
        actual: usize,
    },

    /// The non-membership proof's claimed key does not match the supplied key.
    #[error("non-membership claimed key `{actual}` does not match expected key `{expected}`")]
    KeyMismatch {
        /// The expected key (from the caller).
        expected: String,
        /// The key carried by the proof.
        actual: String,
    },

    /// The non-membership proof's leaf is not the empty-leaf constant.
    #[error("non-membership leaf is not the empty-leaf constant")]
    NonMembershipLeafMismatch,

    /// A store operation failed.
    #[error("store operation failed: {0}")]
    Store(String),

    /// The proof index rejected or could not serve the claim.
    #[error("proof index rejected the claim: {0}")]
    ProofIndex(String),
}

impl From<StateChronicleError> for ProofError {
    fn from(source: StateChronicleError) -> Self {
        match source {
            StateChronicleError::SizeLimitExceeded { .. }
            | StateChronicleError::Canonicalization { .. }
            | StateChronicleError::InvalidDigest(_)
            | StateChronicleError::SignatureVerification(_) => Self::Core(source),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic, clippy::shadow_unrelated)]
mod tests {
    use super::*;
    use statechronicle_core::limits::MAX_ID_LENGTH;

    #[test]
    fn display_messages_are_explicit() {
        assert_eq!(
            ProofError::NotFound.to_string(),
            "no proof available for the requested claim"
        );
        assert_eq!(
            ProofError::MissingOwner.to_string(),
            "claimed state is missing an owner field"
        );
        assert_eq!(
            ProofError::InclusionMismatch.to_string(),
            "state inclusion proof does not verify against the commit state root"
        );
        assert_eq!(
            ProofError::ClaimedStateMismatch.to_string(),
            "claimed state does not match the included leaf"
        );
        assert_eq!(
            ProofError::LeafMismatch.to_string(),
            "inclusion proof leaf does not commit the projected state hash"
        );
        assert_eq!(
            ProofError::UnsupportedSchema(String::from("statechronicle.proof.x")).to_string(),
            "unsupported proof schema `statechronicle.proof.x`"
        );
        assert_eq!(
            ProofError::UnsupportedKind(String::from("jellyfish_v1")).to_string(),
            "unsupported inclusion proof kind `jellyfish_v1`"
        );
        assert_eq!(
            ProofError::NonMembershipLeafMismatch.to_string(),
            "non-membership leaf is not the empty-leaf constant"
        );
        assert_eq!(
            ProofError::KeyMismatch {
                expected: String::from("key-expected"),
                actual: String::from("key-actual"),
            }
            .to_string(),
            "non-membership claimed key `key-actual` does not match expected key `key-expected`"
        );
    }

    #[test]
    fn structured_variants_carry_context() {
        let mismatch = ProofError::SubjectMismatch {
            expected: String::from("account:stexs:player_456"),
            actual: String::from("account:stexs:player_789"),
        };
        let rendered = mismatch.to_string();
        assert!(rendered.contains("account:stexs:player_456"));
        assert!(rendered.contains("account:stexs:player_789"));

        let path = ProofError::InvalidPathLength {
            expected: 256,
            actual: 2,
        };
        assert!(path.to_string().contains("256"));
        assert!(path.to_string().contains("2"));
    }

    #[test]
    fn domain_errors_convert_transparently() {
        let domain = DomainError::InvalidId {
            kind: "commit",
            value: String::from("evt_bad"),
            expected_prefix: String::from("cmt_"),
        };
        let converted = ProofError::from(domain);
        assert!(matches!(converted, ProofError::Domain(_)));
    }

    #[test]
    fn core_errors_map_to_core_variant() {
        let source = StateChronicleError::SignatureVerification(String::from("boom"));
        let converted = ProofError::from(source);
        assert!(matches!(converted, ProofError::Core(_)));

        let source = StateChronicleError::SizeLimitExceeded {
            name: String::from("proof"),
            limit: 8,
            actual: 9,
        };
        let converted = ProofError::from(source);
        assert!(matches!(converted, ProofError::Core(_)));
    }

    #[test]
    fn key_not_found_carries_key_id() {
        let error = ProofError::KeyNotFound(String::from("did:key:z6Mk...#key-1"));
        assert!(error.to_string().contains("did:key:z6Mk...#key-1"));
    }

    #[test]
    fn long_ids_are_accepted_by_display() {
        let id = "x".repeat(MAX_ID_LENGTH);
        let error = ProofError::KeyNotFound(id.clone());
        assert!(error.to_string().contains(&id));
    }
}
