//! Canonical v1 transcript encoding for typed signing and verification paths.

use serde::{Serialize, Serializer};

use crate::limits::{MAX_COMMIT_BYTES, MAX_INTENT_BYTES};
use crate::signature_provider::{KeySetId, SignatureError, SignaturePurpose, SignatureSuiteId};

const DOMAIN: &[u8] = b"statechronicle.signature.v1\0";
const MAX_SCHEMA_BYTES: usize = 128;

struct Bytes<'input>(&'input [u8]);

impl Serialize for Bytes<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_bytes(self.0)
    }
}

/// Builds the signature transcript defined by ADR-009.
///
/// The typed protocol entry point supplies the expected schema and purpose,
/// independently trusted suite and key-set identity, and canonical body bytes.
/// This function encodes those inputs. It performs no policy or body validation.
///
/// # Errors
/// Rejects empty or non-ASCII schemas and inputs exceeding the format limits.
pub fn signature_transcript(
    purpose: SignaturePurpose,
    expected_schema: &str,
    suite: &SignatureSuiteId,
    key_set_id: &KeySetId,
    canonical_body: &[u8],
) -> Result<Vec<u8>, SignatureError> {
    if expected_schema.is_empty()
        || expected_schema.len() > MAX_SCHEMA_BYTES
        || !expected_schema.is_ascii()
    {
        return Err(SignatureError::MalformedEncoding);
    }
    let max_body = match purpose {
        SignaturePurpose::ActorIntent => MAX_INTENT_BYTES,
        SignaturePurpose::TenantCommit | SignaturePurpose::GlobalCheckpoint => MAX_COMMIT_BYTES,
    };
    if canonical_body.len() > max_body {
        return Err(SignatureError::CapacityExceeded);
    }

    // BCS sequences use minimal ULEB128 lengths. Serialize byte slices as
    // byte vectors so the body is copied in one write.
    let fields = (
        Bytes(&[purpose as u8]),
        Bytes(expected_schema.as_bytes()),
        Bytes(suite.as_str().as_bytes()),
        Bytes(&key_set_id.0),
        Bytes(canonical_body),
    );
    let mut transcript = Vec::with_capacity(
        canonical_body.len().saturating_add(
            DOMAIN
                .len()
                .saturating_add(1 + MAX_SCHEMA_BYTES + 64 + 32 + 12),
        ),
    );
    transcript.extend_from_slice(DOMAIN);
    bcs::serialize_into(&mut transcript, &fields)
        .map_err(|_encoding_error| SignatureError::MalformedEncoding)?;
    Ok(transcript)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn matches_independent_adr_vector() {
        let suite = SignatureSuiteId::new("ed25519").unwrap();
        let key = KeySetId([
            0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23,
            24, 25, 26, 27, 28, 29, 30, 31,
        ]);
        let transcript = signature_transcript(
            SignaturePurpose::TenantCommit,
            "statechronicle.commit.v0",
            &suite,
            &key,
            &[42],
        )
        .unwrap();

        assert_eq!(
            hex::encode(transcript),
            concat!(
                "73746174656368726f6e69636c652e7369676e61747572652e763100",
                "0102",
                "1873746174656368726f6e69636c652e636f6d6d69742e7630",
                "0765643235353139",
                "20000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
                "012a",
            )
        );
    }

    #[test]
    fn enforces_intent_body_capacity() {
        let suite = SignatureSuiteId::new("ed25519").unwrap();
        let key = KeySetId([0; 32]);
        let build = |body: &[u8]| {
            signature_transcript(
                SignaturePurpose::ActorIntent,
                "statechronicle.intent.v0",
                &suite,
                &key,
                body,
            )
        };
        assert!(build(&vec![0; MAX_INTENT_BYTES]).is_ok());
        assert_eq!(
            build(&vec![0; MAX_INTENT_BYTES + 1]),
            Err(SignatureError::CapacityExceeded)
        );
    }
}
