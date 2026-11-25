//! Bounded borrowed decoder for the v1 BCS signature envelope.
//!
//! This module validates only envelope framing and bounded byte fields. Typed
//! body decoding and cryptographic dispatch belong to higher protocol layers.

use crate::limits::{MAX_COMMIT_BYTES, MAX_INTENT_BYTES};
use crate::signature_provider::{
    MAX_SIGNATURE_BYTES, SignatureError, SignaturePurpose, validate_suite_id,
};

/// Maximum non-payload framing bytes in a v1 envelope.
///
/// This covers the schema and suite strings, vector length prefixes, and the
/// fixed key-set ID at the current BCS limits.
pub const MAX_V1_ENVELOPE_OVERHEAD_BYTES: usize = 128;

/// Full envelope cap, derived from the largest supported body plus signature
/// and framing limits. Purpose-specific body limits are enforced separately.
pub const MAX_V1_ENVELOPE_BYTES: usize =
    MAX_COMMIT_BYTES + MAX_SIGNATURE_BYTES + MAX_V1_ENVELOPE_OVERHEAD_BYTES;

/// Full mixed-record cap including its one-byte version discriminator.
pub const MAX_V1_MIXED_RECORD_BYTES: usize = MAX_V1_ENVELOPE_BYTES + 1;

const ENVELOPE_SCHEMA: &str = "statechronicle.signed.v1";

/// Borrowed fields from a validated v1 envelope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct V1SignedEnvelope<'input> {
    /// Canonical BCS encoding of the purpose-specific body.
    pub body: &'input [u8],
    /// Syntactically valid suite identifier. Policy still decides whether it is trusted.
    pub suite_id: &'input str,
    /// Registry-assigned key-set identity.
    pub key_set_id: &'input [u8; 32],
    /// Bounded signature payload. Suite-specific decoding is performed later.
    pub signature: &'input [u8],
}

/// Decodes a standalone v1 envelope without copying its body or signature.
///
/// The purpose selects the body-size limit. This function does not decode or
/// validate the purpose-specific body contents.
///
/// # Errors
/// Returns [`SignatureError::MalformedEncoding`] for malformed, nonminimal,
/// truncated, trailing, or invalid identifier bytes, and
/// [`SignatureError::CapacityExceeded`] for size-limit violations.
pub fn decode_v1_envelope<'input>(
    input: &'input [u8],
    purpose: SignaturePurpose,
) -> Result<V1SignedEnvelope<'input>, SignatureError> {
    check_record_size(input.len(), MAX_V1_ENVELOPE_BYTES)?;
    let max_body = body_limit(purpose);
    let mut cursor = 0;

    let schema = read_string(input, &mut cursor, ENVELOPE_SCHEMA.len())?;
    if schema != ENVELOPE_SCHEMA {
        return Err(SignatureError::MalformedEncoding);
    }
    let body = read_byte_vector(input, &mut cursor, max_body)?;
    let suite_id = read_string(input, &mut cursor, 64)?;
    validate_suite_id(suite_id)?;
    let key_bytes = input
        .get(
            cursor
                ..cursor
                    .checked_add(32)
                    .ok_or(SignatureError::MalformedEncoding)?,
        )
        .ok_or(SignatureError::MalformedEncoding)?;
    let key_set_id: &[u8; 32] = key_bytes
        .try_into()
        .map_err(|_error| SignatureError::MalformedEncoding)?;
    cursor = cursor
        .checked_add(32)
        .ok_or(SignatureError::MalformedEncoding)?;
    let signature = read_byte_vector(input, &mut cursor, MAX_SIGNATURE_BYTES)?;
    if cursor != input.len() {
        return Err(SignatureError::MalformedEncoding);
    }

    Ok(V1SignedEnvelope {
        body,
        suite_id,
        key_set_id,
        signature,
    })
}

/// Decodes a v1 record from the mixed-version record format.
///
/// This codec accepts only discriminator `0x01`; `0x00` is a legacy record and
/// must be handled by a separately selected legacy entry point. No fallback is
/// attempted here.
///
/// # Errors
/// Returns [`SignatureError::MalformedEncoding`] for a non-v1 discriminator or
/// malformed envelope, and [`SignatureError::CapacityExceeded`] for oversized input.
pub fn decode_mixed_v1_record<'input>(
    input: &'input [u8],
    purpose: SignaturePurpose,
) -> Result<V1SignedEnvelope<'input>, SignatureError> {
    check_record_size(input.len(), MAX_V1_MIXED_RECORD_BYTES)?;
    match input.first() {
        Some(0x01) => decode_v1_envelope(
            input.get(1..).ok_or(SignatureError::MalformedEncoding)?,
            purpose,
        ),
        Some(0x00) | Some(_) | None => Err(SignatureError::MalformedEncoding),
    }
}

pub(crate) const fn body_limit(purpose: SignaturePurpose) -> usize {
    match purpose {
        SignaturePurpose::ActorIntent => MAX_INTENT_BYTES,
        SignaturePurpose::TenantCommit | SignaturePurpose::GlobalCheckpoint => MAX_COMMIT_BYTES,
    }
}

const fn check_record_size(actual: usize, limit: usize) -> Result<(), SignatureError> {
    if actual > limit {
        return Err(SignatureError::CapacityExceeded);
    }
    Ok(())
}

fn read_string<'input>(
    input: &'input [u8],
    cursor: &mut usize,
    max_len: usize,
) -> Result<&'input str, SignatureError> {
    let bytes = read_byte_vector(input, cursor, max_len)?;
    if !bytes.is_ascii() {
        return Err(SignatureError::MalformedEncoding);
    }
    std::str::from_utf8(bytes).map_err(|_error| SignatureError::MalformedEncoding)
}

fn read_byte_vector<'input>(
    input: &'input [u8],
    cursor: &mut usize,
    max_len: usize,
) -> Result<&'input [u8], SignatureError> {
    let len = read_uleb128(input, cursor)?;
    if len > max_len {
        return Err(SignatureError::CapacityExceeded);
    }
    let end = cursor
        .checked_add(len)
        .ok_or(SignatureError::MalformedEncoding)?;
    let bytes = input
        .get(*cursor..end)
        .ok_or(SignatureError::MalformedEncoding)?;
    *cursor = end;
    Ok(bytes)
}

/// Reads a canonical minimal BCS ULEB128 `usize` without allocation.
fn read_uleb128(input: &[u8], cursor: &mut usize) -> Result<usize, SignatureError> {
    let mut value = 0usize;
    let mut shift = 0u32;
    let mut count = 0usize;
    loop {
        let byte = *input
            .get(*cursor)
            .ok_or(SignatureError::MalformedEncoding)?;
        *cursor = cursor
            .checked_add(1)
            .ok_or(SignatureError::MalformedEncoding)?;
        let payload = usize::from(byte & 0x7f);
        if shift >= usize::BITS || payload > (usize::MAX >> shift) {
            return Err(SignatureError::MalformedEncoding);
        }
        value |= payload << shift;
        count = count
            .checked_add(1)
            .ok_or(SignatureError::MalformedEncoding)?;
        if byte & 0x80 == 0 {
            if count > 1 && payload == 0 {
                return Err(SignatureError::MalformedEncoding);
            }
            return Ok(value);
        }
        if count >= 10 {
            return Err(SignatureError::MalformedEncoding);
        }
        shift = shift
            .checked_add(7)
            .ok_or(SignatureError::MalformedEncoding)?;
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;

    const PURPOSE: SignaturePurpose = SignaturePurpose::TenantCommit;

    #[test]
    fn decodes_borrowed_independent_envelope_fixture() {
        let bytes =
            include_bytes!("../tests/fixtures/signature-v1/positive_v1_envelope_layout.bin");
        let decoded = decode_v1_envelope(bytes, PURPOSE).unwrap();
        assert_eq!(decoded.body, &[0x2a]);
        assert_eq!(decoded.suite_id, "ed25519");
        assert_eq!(decoded.key_set_id, &(0u8..32).collect::<Vec<_>>()[..]);
        assert_eq!(decoded.signature, &(0u8..64).collect::<Vec<_>>()[..]);
        assert!(bytes.as_ptr_range().contains(&decoded.body.as_ptr()));
        assert!(bytes.as_ptr_range().contains(&decoded.signature.as_ptr()));
    }

    #[test]
    fn decodes_v1_mixed_record_without_legacy_fallback() {
        let bytes =
            include_bytes!("../tests/fixtures/signature-v1/positive_mixed_record_v1_layout.bin");
        let decoded = decode_mixed_v1_record(bytes, PURPOSE).unwrap();
        assert_eq!(decoded.body, &[0x2a]);
        assert_eq!(decoded.suite_id, "ed25519");
    }

    #[test]
    fn rejects_malformed_layout_fixtures() {
        for bytes in [
            &include_bytes!(
                "../tests/fixtures/signature-v1/malformed_nonminimal_schema_length.bin"
            )[..],
            &include_bytes!(
                "../tests/fixtures/signature-v1/malformed_truncated_signature_vector.bin"
            )[..],
            &include_bytes!("../tests/fixtures/signature-v1/malformed_trailing_envelope_byte.bin")
                [..],
        ] {
            assert!(decode_v1_envelope(bytes, PURPOSE).is_err());
        }
        assert!(
            decode_mixed_v1_record(
                include_bytes!(
                    "../tests/fixtures/signature-v1/malformed_unknown_outer_version.bin"
                ),
                PURPOSE
            )
            .is_err()
        );
        // A legacy discriminator is deliberately not dispatched from this v1 decoder.
        assert!(decode_mixed_v1_record(&[0x00], PURPOSE).is_err());
    }

    #[test]
    fn checks_purpose_specific_body_limit_before_copying() {
        let mut input = Vec::new();
        input.extend_from_slice(&[0x18]);
        input.extend_from_slice(ENVELOPE_SCHEMA.as_bytes());
        input.extend_from_slice(&[0x81, 0x80, 0x04]); // 65,537, minimal ULEB128.
        input.resize(input.len() + MAX_INTENT_BYTES + 1, 0);
        assert_eq!(
            decode_v1_envelope(&input, SignaturePurpose::ActorIntent),
            Err(SignatureError::CapacityExceeded)
        );
    }

    #[test]
    fn rejects_oversized_full_record_before_parsing() {
        let input = vec![0xff; MAX_V1_ENVELOPE_BYTES + 1];
        assert_eq!(
            decode_v1_envelope(&input, PURPOSE),
            Err(SignatureError::CapacityExceeded)
        );
    }

    #[test]
    fn rejects_signature_vector_over_limit_before_borrowing_payload() {
        let mut input = Vec::new();
        input.extend_from_slice(&[0x18]);
        input.extend_from_slice(ENVELOPE_SCHEMA.as_bytes());
        input.extend_from_slice(&[0x00]); // empty body
        input.extend_from_slice(&[0x07]);
        input.extend_from_slice(b"ed25519");
        input.extend_from_slice(&[0; 32]);
        let signature_len = MAX_SIGNATURE_BYTES + 1;
        input.extend_from_slice(&[0x81, 0x80, 0x01]); // 16,385
        input.resize(input.len() + signature_len, 0);
        assert_eq!(
            decode_v1_envelope(&input, PURPOSE),
            Err(SignatureError::CapacityExceeded)
        );
    }
}
