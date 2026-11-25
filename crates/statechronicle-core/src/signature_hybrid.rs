//! Canonical component framing for the ADR-009 Ed25519 and ML-DSA-65 suite.

use crate::signature::SIGNATURE_BYTE_LEN;
use crate::signature_provider::SignatureError;

/// Size of a pure ML-DSA-65 signature.
pub const ML_DSA_65_SIGNATURE_BYTES: usize = 3309;

const FIRST_HEADER: &[u8] = &[1, 2, 1, 64];
const SECOND_HEADER: &[u8] = &[2, 0xed, 0x19];

/// Size of the complete Ed25519 and ML-DSA-65 hybrid signature.
pub const HYBRID_SIGNATURE_BYTES: usize =
    FIRST_HEADER.len() + SIGNATURE_BYTE_LEN + SECOND_HEADER.len() + ML_DSA_65_SIGNATURE_BYTES;

/// The two required hybrid signature components in canonical order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HybridSignature<'signature> {
    /// Ed25519 signature bytes.
    pub ed25519: &'signature [u8; SIGNATURE_BYTE_LEN],
    /// ML-DSA-65 signature bytes.
    pub ml_dsa_65: &'signature [u8; ML_DSA_65_SIGNATURE_BYTES],
}

impl<'signature> HybridSignature<'signature> {
    /// Borrows both components after validating the entire payload.
    ///
    /// # Errors
    /// Rejects invalid length, version, member count, order, or length prefixes.
    pub fn decode(payload: &'signature [u8]) -> Result<Self, SignatureError> {
        let (ed25519, remaining) = payload
            .strip_prefix(FIRST_HEADER)
            .and_then(<[u8]>::split_first_chunk::<SIGNATURE_BYTE_LEN>)
            .ok_or(SignatureError::MalformedEncoding)?;
        let ml_dsa_65 = remaining
            .strip_prefix(SECOND_HEADER)
            .and_then(|member| member.try_into().ok())
            .ok_or(SignatureError::MalformedEncoding)?;
        Ok(Self { ed25519, ml_dsa_65 })
    }

    /// Writes the canonical payload, leaving extra output capacity untouched.
    ///
    /// # Errors
    /// Returns a capacity error without writing if the buffer is too small.
    pub fn encode_into(&self, output: &mut [u8]) -> Result<usize, SignatureError> {
        let destination = output
            .get_mut(..HYBRID_SIGNATURE_BYTES)
            .ok_or(SignatureError::CapacityExceeded)?;
        let mut writer = destination;
        std::io::Write::write_all(&mut writer, FIRST_HEADER)
            .and_then(|()| std::io::Write::write_all(&mut writer, self.ed25519))
            .and_then(|()| std::io::Write::write_all(&mut writer, SECOND_HEADER))
            .and_then(|()| std::io::Write::write_all(&mut writer, self.ml_dsa_65))
            .map_err(|_capacity_error| SignatureError::CapacityExceeded)?;
        Ok(HYBRID_SIGNATURE_BYTES)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    const VALID: &[u8] =
        include_bytes!("../tests/fixtures/signature-v1/positive_hybrid_payload_layout.bin");
    const DUPLICATE: &[u8] =
        include_bytes!("../tests/fixtures/signature-v1/malformed_hybrid_duplicate_member_tag.bin");
    const TRUNCATED: &[u8] =
        include_bytes!("../tests/fixtures/signature-v1/malformed_hybrid_truncated_member.bin");

    #[test]
    fn independent_hybrid_layout_round_trips() {
        let signature = HybridSignature::decode(VALID).unwrap();
        let mut output = [0; HYBRID_SIGNATURE_BYTES];
        assert_eq!(signature.encode_into(&mut output).unwrap(), VALID.len());
        assert_eq!(output.as_slice(), VALID);
        let mut short = [42; HYBRID_SIGNATURE_BYTES - 1];
        assert_eq!(
            signature.encode_into(&mut short),
            Err(SignatureError::CapacityExceeded)
        );
        assert_eq!(short, [42; HYBRID_SIGNATURE_BYTES - 1]);
    }

    #[test]
    fn rejects_malformed_components_and_trailing_bytes() {
        let mut trailing = VALID.to_vec();
        trailing.push(0);
        for input in [DUPLICATE, TRUNCATED, &trailing] {
            assert_eq!(
                HybridSignature::decode(input),
                Err(SignatureError::MalformedEncoding)
            );
        }
    }
}
