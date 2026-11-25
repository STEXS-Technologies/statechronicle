//! Canonical component framing for the ADR-009 Ed25519 and ML-DSA-65 suite.

use crate::signature::SIGNATURE_BYTE_LEN;
use crate::signature_provider::{
    KeySetId, ProviderBinding, SignatureError, SignaturePurpose, SignatureSuiteId, Signer,
    SigningRequest, VerificationRequest, Verifier,
};

/// Suite identifier for the ordered Ed25519 and ML-DSA-65 hybrid.
pub const HYBRID_SUITE_ID: &str = "hybrid-ed25519-ml-dsa-65.v1";

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

/// A member signer and key-set identity selected together by trusted policy.
pub struct HybridSignerMember<'provider> {
    /// Provider configured for the member signing key.
    pub signer: &'provider dyn Signer,
    /// Registry identity for the member key set.
    pub key_set_id: KeySetId,
}

impl HybridSignerMember<'_> {
    fn sign_into(
        &self,
        request: SigningRequest<'_>,
        output: &mut [u8],
    ) -> Result<(), SignatureError> {
        let written = self.signer.sign_into(
            SigningRequest {
                suite: self.signer.suite(),
                key_set_id: &self.key_set_id,
                ..request
            },
            output,
        )?;
        if written == output.len() {
            Ok(())
        } else {
            Err(SignatureError::MalformedEncoding)
        }
    }
}

/// Signs one composite transcript with both pinned member keys.
pub struct HybridSigner<'provider> {
    binding: ProviderBinding,
    ed25519: HybridSignerMember<'provider>,
    ml_dsa_65: HybridSignerMember<'provider>,
}

impl<'provider> HybridSigner<'provider> {
    /// Binds the ordered member providers to the trusted composite identity.
    ///
    /// # Errors
    /// Rejects providers whose suites differ from the required member suites.
    pub fn new(
        key_set_id: KeySetId,
        purpose: SignaturePurpose,
        ed25519: HybridSignerMember<'provider>,
        ml_dsa_65: HybridSignerMember<'provider>,
    ) -> Result<Self, SignatureError> {
        check_member_suites(ed25519.signer.suite(), ml_dsa_65.signer.suite())?;
        Ok(Self {
            binding: ProviderBinding::new(HYBRID_SUITE_ID, key_set_id, purpose),
            ed25519,
            ml_dsa_65,
        })
    }
}

impl Signer for HybridSigner<'_> {
    fn suite(&self) -> &SignatureSuiteId {
        &self.binding.suite
    }

    fn sign_into(
        &self,
        request: SigningRequest<'_>,
        output: &mut [u8],
    ) -> Result<usize, SignatureError> {
        self.binding.check(request)?;
        if output.len() < HYBRID_SIGNATURE_BYTES {
            return Err(SignatureError::CapacityExceeded);
        }

        let mut ed25519 = [0; SIGNATURE_BYTE_LEN];
        let mut ml_dsa_65 = [0; ML_DSA_65_SIGNATURE_BYTES];
        self.ed25519.sign_into(request, &mut ed25519)?;
        self.ml_dsa_65.sign_into(request, &mut ml_dsa_65)?;
        HybridSignature {
            ed25519: &ed25519,
            ml_dsa_65: &ml_dsa_65,
        }
        .encode_into(output)
    }
}

fn check_member_suites(
    ed25519: &SignatureSuiteId,
    ml_dsa_65: &SignatureSuiteId,
) -> Result<(), SignatureError> {
    if ed25519.as_str() == "ed25519" && ml_dsa_65.as_str() == "ml-dsa-65" {
        Ok(())
    } else {
        Err(SignatureError::UnsupportedSuite)
    }
}

/// A member verifier and key-set identity selected together by trusted policy.
pub struct HybridVerifierMember<'provider> {
    /// Provider configured for this member's public key.
    pub verifier: &'provider dyn Verifier,
    /// Registry identity for the member key set.
    pub key_set_id: KeySetId,
}

impl HybridVerifierMember<'_> {
    fn verify(&self, request: SigningRequest<'_>, signature: &[u8]) -> Result<(), SignatureError> {
        self.verifier.verify(VerificationRequest {
            request: SigningRequest {
                suite: self.verifier.suite(),
                key_set_id: &self.key_set_id,
                ..request
            },
            signature,
        })
    }
}

/// Verifies both members against independently resolved keys and one transcript.
pub struct HybridVerifier<'provider> {
    binding: ProviderBinding,
    ed25519: HybridVerifierMember<'provider>,
    ml_dsa_65: HybridVerifierMember<'provider>,
}

impl<'provider> HybridVerifier<'provider> {
    /// Binds the ordered member providers to the trusted composite identity.
    ///
    /// # Errors
    /// Rejects providers with suites different from the required member suites.
    pub fn new(
        key_set_id: KeySetId,
        purpose: SignaturePurpose,
        ed25519: HybridVerifierMember<'provider>,
        ml_dsa_65: HybridVerifierMember<'provider>,
    ) -> Result<Self, SignatureError> {
        check_member_suites(ed25519.verifier.suite(), ml_dsa_65.verifier.suite())?;
        Ok(Self {
            binding: ProviderBinding::new(HYBRID_SUITE_ID, key_set_id, purpose),
            ed25519,
            ml_dsa_65,
        })
    }
}

impl Verifier for HybridVerifier<'_> {
    fn suite(&self) -> &SignatureSuiteId {
        &self.binding.suite
    }

    fn verify(&self, request: VerificationRequest<'_>) -> Result<(), SignatureError> {
        self.binding.check(request.request)?;
        let signature = HybridSignature::decode(request.signature)?;
        self.ed25519.verify(request.request, signature.ed25519)?;
        self.ml_dsa_65.verify(request.request, signature.ml_dsa_65)
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

    // Test double for the provider contract. It performs no cryptography.
    struct TestMlDsa {
        suite: SignatureSuiteId,
        expected_key: KeySetId,
    }

    impl Verifier for TestMlDsa {
        fn suite(&self) -> &SignatureSuiteId {
            &self.suite
        }

        fn verify(&self, request: VerificationRequest<'_>) -> Result<(), SignatureError> {
            if request.request.key_set_id == &self.expected_key
                && request.request.purpose == SignaturePurpose::TenantCommit
                && request.request.transcript == b"same hybrid transcript"
                && request.signature == [7; ML_DSA_65_SIGNATURE_BYTES]
            {
                Ok(())
            } else {
                Err(SignatureError::InvalidSignature)
            }
        }
    }

    struct TestSigner {
        suite: SignatureSuiteId,
        result: Result<usize, SignatureError>,
    }

    impl Signer for TestSigner {
        fn suite(&self) -> &SignatureSuiteId {
            &self.suite
        }

        fn sign_into(
            &self,
            _request: SigningRequest<'_>,
            output: &mut [u8],
        ) -> Result<usize, SignatureError> {
            output.fill(7);
            self.result
        }
    }

    #[test]
    fn hybrid_signer_publishes_only_complete_output() {
        use crate::signature_provider::Ed25519Signer;
        use ed25519_dalek::SigningKey;

        let signing_key = SigningKey::from_bytes(&[9; 32]);
        let composite_key = KeySetId([1; 32]);
        let purpose = SignaturePurpose::TenantCommit;
        let ed_signer = Ed25519Signer::new(&signing_key, KeySetId([2; 32]), purpose);
        for (member_result, expected) in [
            (Ok(ML_DSA_65_SIGNATURE_BYTES), Ok(HYBRID_SIGNATURE_BYTES)),
            (Ok(1), Err(SignatureError::MalformedEncoding)),
            (
                Err(SignatureError::ProviderUnavailable),
                Err(SignatureError::ProviderUnavailable),
            ),
        ] {
            let pq = TestSigner {
                suite: SignatureSuiteId::new("ml-dsa-65").unwrap(),
                result: member_result,
            };
            let signer = HybridSigner::new(
                composite_key,
                purpose,
                HybridSignerMember {
                    signer: &ed_signer,
                    key_set_id: KeySetId([2; 32]),
                },
                HybridSignerMember {
                    signer: &pq,
                    key_set_id: KeySetId([3; 32]),
                },
            )
            .unwrap();
            let mut output = [42; HYBRID_SIGNATURE_BYTES];
            let request = SigningRequest {
                suite: signer.suite(),
                key_set_id: &composite_key,
                purpose,
                transcript: b"same hybrid transcript",
            };
            assert_eq!(signer.sign_into(request, &mut output), expected);
            if expected.is_ok() {
                let signature = HybridSignature::decode(&output).unwrap();
                assert_eq!(signature.ml_dsa_65, &[7; ML_DSA_65_SIGNATURE_BYTES]);
            } else {
                assert_eq!(output, [42; HYBRID_SIGNATURE_BYTES]);
            }
        }
    }

    #[test]
    fn hybrid_verification_requires_both_pinned_members() {
        use crate::signature;
        use crate::signature_provider::Ed25519Verifier;
        use ed25519_dalek::SigningKey;

        let signing_key = SigningKey::from_bytes(&[9; 32]);
        let verifying_key = signing_key.verifying_key();
        let composite_key = KeySetId([1; 32]);
        let ed_key = KeySetId([2; 32]);
        let pq_key = KeySetId([3; 32]);
        let purpose = SignaturePurpose::TenantCommit;
        let ed_verifier = Ed25519Verifier::new(&verifying_key, ed_key, purpose);
        let pq = TestMlDsa {
            suite: SignatureSuiteId::new("ml-dsa-65").unwrap(),
            expected_key: pq_key,
        };
        let hybrid = HybridVerifier::new(
            composite_key,
            purpose,
            HybridVerifierMember {
                verifier: &ed_verifier,
                key_set_id: ed_key,
            },
            HybridVerifierMember {
                verifier: &pq,
                key_set_id: pq_key,
            },
        )
        .unwrap();
        let transcript = b"same hybrid transcript";
        let ed_signature = signature::sign(transcript, &signing_key);
        let request = SigningRequest {
            suite: hybrid.suite(),
            key_set_id: &composite_key,
            purpose,
            transcript,
        };

        let verify = |ed25519, ml_dsa_65| {
            let mut payload = [0; HYBRID_SIGNATURE_BYTES];
            HybridSignature { ed25519, ml_dsa_65 }
                .encode_into(&mut payload)
                .unwrap();
            hybrid.verify(VerificationRequest {
                request,
                signature: &payload,
            })
        };
        assert!(verify(ed_signature.as_bytes(), &[7; ML_DSA_65_SIGNATURE_BYTES]).is_ok());
        assert_eq!(
            verify(ed_signature.as_bytes(), &[8; ML_DSA_65_SIGNATURE_BYTES]),
            Err(SignatureError::InvalidSignature)
        );
        assert_eq!(
            verify(&[0; SIGNATURE_BYTE_LEN], &[7; ML_DSA_65_SIGNATURE_BYTES]),
            Err(SignatureError::InvalidSignature)
        );
    }

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
