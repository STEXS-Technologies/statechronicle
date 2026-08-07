//! BCS canonical serialization of protocol objects (ADR-004).
//!
//! BCS (Binary Canonical Serialization) produces a single canonical byte
//! encoding per value with no configuration: length-prefixed sequences,
//! fixed-width little-endian integers, no floats, no key names. Determinism is
//! by construction, which makes it the foundation for content-addressing
//! intents, events, and commits. JSON remains the HTTP API logical view, but
//! all hashed and signed objects use the BCS canonical bytes. Signed objects
//! use an explicit envelope body (`Obj { body, signature }`) so a signature
//! covers only `bcs::to_bytes(&body)`, never the signature field (ADR-004 §2).

use serde::Serialize;

use crate::digest::{ContentDigest, hash_bytes};
use crate::error::StateChronicleError;

/// Serializes `value` into its canonical BCS byte representation (ADR-004).
///
/// Deterministic by construction: the same value always produces the same
/// bytes, regardless of process or platform. Because BCS is minimal, a decoded
/// value re-encodes to exactly the original input bytes.
///
/// # Errors
///
/// Returns [`StateChronicleError::Canonicalization`] when BCS serialization
/// fails, e.g. for a float (structurally banned by the protocol), a
/// non-minimal length prefix, or a container nesting deeper than the BCS
/// depth limit.
pub fn canonicalize<T: Serialize + ?Sized>(value: &T) -> Result<Vec<u8>, StateChronicleError> {
    let bytes = bcs::to_bytes(value)?;
    Ok(bytes)
}

/// Canonicalizes `value` and computes its `sha256:` content digest.
///
/// Equivalent to `hash_bytes(canonicalize(value)?)` — the digest is over the
/// BCS canonical bytes, which is the protocol's `digest()` for hashed and
/// signed objects (ADR-004 §4).
///
/// # Errors
///
/// Returns [`StateChronicleError::Canonicalization`] when BCS serialization
/// fails (see [`canonicalize`]).
pub fn canonicalize_and_digest<T: Serialize + ?Sized>(
    value: &T,
) -> Result<ContentDigest, StateChronicleError> {
    let bytes = canonicalize(value)?;
    Ok(hash_bytes(&bytes))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::{canonicalize, canonicalize_and_digest};
    use crate::digest::hash_bytes;
    use crate::error::StateChronicleError;

    #[derive(serde::Deserialize, serde::Serialize, Debug, PartialEq, Eq)]
    struct TestRecord {
        tenant_id: String,
        resource_id: String,
        version: u64,
        tags: Vec<String>,
    }

    fn sample_record() -> TestRecord {
        TestRecord {
            tenant_id: String::from("tenant:acme"),
            resource_id: String::from("asset:sword_001"),
            version: 41,
            tags: vec![String::from("rare"), String::from("bound")],
        }
    }

    #[test]
    fn canonicalize_is_deterministic() {
        let record = sample_record();
        let first = canonicalize(&record).unwrap();
        let second = canonicalize(&record).unwrap();

        assert_eq!(first, second);
        assert!(!first.is_empty());
    }

    #[test]
    fn canonicalize_roundtrips_through_bcs() {
        let record = sample_record();
        let bytes = canonicalize(&record).unwrap();
        let decoded: TestRecord = bcs::from_bytes(&bytes).unwrap();

        assert_eq!(decoded, record);
    }

    #[test]
    fn canonicalize_and_digest_matches_hash_of_canonical_bytes() {
        let record = sample_record();
        let bytes = canonicalize(&record).unwrap();
        let digest = canonicalize_and_digest(&record).unwrap();

        assert_eq!(digest.as_bytes(), hash_bytes(&bytes).as_bytes());
    }

    #[test]
    fn canonicalize_reports_bcs_errors() {
        #[derive(serde::Serialize)]
        struct FloatRecord {
            amount: f64,
        }

        let record = FloatRecord { amount: 1.5 };
        let error = canonicalize(&record).unwrap_err();

        assert!(matches!(
            error,
            StateChronicleError::Canonicalization { .. }
        ));
    }
}
