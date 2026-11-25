//! Interoperability checks for optional external signature providers.
use statechronicle_core::signature_provider::{
    KeySetId, SignatureError, SignaturePurpose, SignatureSuiteId, Signer, SigningRequest,
    VerificationRequest, Verifier,
};

pub struct MlDsa65Signer {
    key: ml_dsa::SigningKey<ml_dsa::MlDsa65>,
    key_set_id: KeySetId,
    purpose: SignaturePurpose,
}
impl MlDsa65Signer {
    pub fn new(
        key: ml_dsa::SigningKey<ml_dsa::MlDsa65>,
        key_set_id: KeySetId,
        purpose: SignaturePurpose,
    ) -> Self {
        Self {
            key,
            key_set_id,
            purpose,
        }
    }
}
impl Signer for MlDsa65Signer {
    fn suite(&self) -> &SignatureSuiteId {
        static SUITE: std::sync::OnceLock<SignatureSuiteId> = std::sync::OnceLock::new();
        SUITE.get_or_init(|| SignatureSuiteId::new("ml-dsa-65").expect("static suite is valid"))
    }
    fn sign_into(
        &self,
        request: SigningRequest<'_>,
        output: &mut [u8],
    ) -> Result<usize, SignatureError> {
        check_binding(request, self.suite(), self.key_set_id, self.purpose)?;
        if output.len() < 3309 {
            return Err(SignatureError::CapacityExceeded);
        }
        use ml_dsa::Signer as _;
        let signature = self
            .key
            .try_sign(request.transcript)
            .map_err(|_| SignatureError::ProviderUnavailable)?;
        let encoded = signature.encode();
        output
            .get_mut(..3309)
            .ok_or(SignatureError::CapacityExceeded)?
            .copy_from_slice(encoded.as_slice());
        Ok(3309)
    }
}
pub struct MlDsa65Verifier {
    key: ml_dsa::VerifyingKey<ml_dsa::MlDsa65>,
    key_set_id: KeySetId,
    purpose: SignaturePurpose,
}
impl MlDsa65Verifier {
    pub fn new(
        key: ml_dsa::VerifyingKey<ml_dsa::MlDsa65>,
        key_set_id: KeySetId,
        purpose: SignaturePurpose,
    ) -> Self {
        Self {
            key,
            key_set_id,
            purpose,
        }
    }
}
impl Verifier for MlDsa65Verifier {
    fn suite(&self) -> &SignatureSuiteId {
        static SUITE: std::sync::OnceLock<SignatureSuiteId> = std::sync::OnceLock::new();
        SUITE.get_or_init(|| SignatureSuiteId::new("ml-dsa-65").expect("static suite is valid"))
    }
    fn verify(&self, request: VerificationRequest<'_>) -> Result<(), SignatureError> {
        check_binding(request.request, self.suite(), self.key_set_id, self.purpose)?;
        if request.signature.len() != 3309 {
            return Err(SignatureError::MalformedEncoding);
        }
        let signature = ml_dsa::Signature::<ml_dsa::MlDsa65>::try_from(request.signature)
            .map_err(|_| SignatureError::MalformedEncoding)?;
        use ml_dsa::Verifier as _;
        self.key
            .verify(request.request.transcript, &signature)
            .map_err(|_| SignatureError::InvalidSignature)
    }
}

pub struct SlhDsaSha2_128sSigner {
    key: slh_dsa::SigningKey<slh_dsa::Sha2_128s>,
    key_set_id: KeySetId,
    purpose: SignaturePurpose,
}
impl SlhDsaSha2_128sSigner {
    pub fn new(
        key: slh_dsa::SigningKey<slh_dsa::Sha2_128s>,
        key_set_id: KeySetId,
        purpose: SignaturePurpose,
    ) -> Self {
        Self {
            key,
            key_set_id,
            purpose,
        }
    }
}
impl Signer for SlhDsaSha2_128sSigner {
    fn suite(&self) -> &SignatureSuiteId {
        static SUITE: std::sync::OnceLock<SignatureSuiteId> = std::sync::OnceLock::new();
        SUITE.get_or_init(|| {
            SignatureSuiteId::new("slh-dsa-sha2-128s").expect("static suite is valid")
        })
    }
    fn sign_into(
        &self,
        request: SigningRequest<'_>,
        output: &mut [u8],
    ) -> Result<usize, SignatureError> {
        check_binding(request, self.suite(), self.key_set_id, self.purpose)?;
        if output.len() < 7856 {
            return Err(SignatureError::CapacityExceeded);
        }
        use slh_dsa::signature::Signer as _;
        let signature = self
            .key
            .try_sign(request.transcript)
            .map_err(|_| SignatureError::ProviderUnavailable)?;
        let encoded = signature.to_bytes();
        output
            .get_mut(..7856)
            .ok_or(SignatureError::CapacityExceeded)?
            .copy_from_slice(encoded.as_slice());
        Ok(7856)
    }
}
pub struct SlhDsaSha2_128sVerifier {
    key: slh_dsa::VerifyingKey<slh_dsa::Sha2_128s>,
    key_set_id: KeySetId,
    purpose: SignaturePurpose,
}
impl SlhDsaSha2_128sVerifier {
    pub fn new(
        key: slh_dsa::VerifyingKey<slh_dsa::Sha2_128s>,
        key_set_id: KeySetId,
        purpose: SignaturePurpose,
    ) -> Self {
        Self {
            key,
            key_set_id,
            purpose,
        }
    }
}
impl Verifier for SlhDsaSha2_128sVerifier {
    fn suite(&self) -> &SignatureSuiteId {
        static SUITE: std::sync::OnceLock<SignatureSuiteId> = std::sync::OnceLock::new();
        SUITE.get_or_init(|| {
            SignatureSuiteId::new("slh-dsa-sha2-128s").expect("static suite is valid")
        })
    }
    fn verify(&self, request: VerificationRequest<'_>) -> Result<(), SignatureError> {
        check_binding(request.request, self.suite(), self.key_set_id, self.purpose)?;
        if request.signature.len() != 7856 {
            return Err(SignatureError::MalformedEncoding);
        }
        let signature = slh_dsa::Signature::<slh_dsa::Sha2_128s>::try_from(request.signature)
            .map_err(|_| SignatureError::MalformedEncoding)?;
        use slh_dsa::signature::Verifier as _;
        self.key
            .verify(request.request.transcript, &signature)
            .map_err(|_| SignatureError::InvalidSignature)
    }
}

fn check_binding(
    request: SigningRequest<'_>,
    suite: &SignatureSuiteId,
    key_set_id: KeySetId,
    purpose: SignaturePurpose,
) -> Result<(), SignatureError> {
    if request.suite != suite {
        return Err(SignatureError::UnsupportedSuite);
    }
    if *request.key_set_id != key_set_id {
        return Err(SignatureError::KeyUnavailable);
    }
    if request.purpose != purpose {
        return Err(SignatureError::UnauthorizedSuite);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use statechronicle_core::signature_transcript::signature_transcript;

    fn roundtrip<S: Signer, V: Verifier>(
        signer: &S,
        verifier: &V,
        key_set: &KeySetId,
        purpose: SignaturePurpose,
        schema: &str,
        signature_len: usize,
    ) {
        let suite = signer.suite();
        let transcript = signature_transcript(
            purpose,
            schema,
            suite,
            key_set,
            b"independent protocol body",
        )
        .unwrap();
        let request = SigningRequest {
            suite,
            key_set_id: key_set,
            purpose,
            transcript: &transcript,
        };
        let mut signature = vec![0; signature_len];
        assert_eq!(
            signer.sign_into(request, &mut signature).unwrap(),
            signature_len
        );
        verifier
            .verify(VerificationRequest {
                request,
                signature: &signature,
            })
            .unwrap();
        let bad_transcript =
            signature_transcript(purpose, schema, suite, key_set, b"tampered body").unwrap();
        let bad_request = SigningRequest {
            suite,
            key_set_id: key_set,
            purpose,
            transcript: &bad_transcript,
        };
        assert!(
            verifier
                .verify(VerificationRequest {
                    request: bad_request,
                    signature: &signature
                })
                .is_err()
        );
    }

    #[test]
    fn ml_dsa_65_signer_and_verifier_roundtrip() {
        use ml_dsa::Keypair as _;
        let seed: ml_dsa::Seed = [0x42; 32].into();
        let signing_key = ml_dsa::SigningKey::<ml_dsa::MlDsa65>::from_seed(&seed);
        let verifying_key = signing_key.verifying_key();
        let key_set = KeySetId([0x33; 32]);
        let purpose = SignaturePurpose::TenantCommit;
        let signer = MlDsa65Signer::new(signing_key, key_set, purpose);
        let verifier = MlDsa65Verifier::new(verifying_key, key_set, purpose);
        roundtrip(
            &signer,
            &verifier,
            &key_set,
            purpose,
            "statechronicle.commit.v0",
            3309,
        );
        let aws_verifier =
            AwsLcMlDsa65Verifier::new(verifier.key.encode().to_vec(), key_set, purpose);
        roundtrip(
            &signer,
            &aws_verifier,
            &key_set,
            purpose,
            "statechronicle.commit.v0",
            3309,
        );
    }

    #[test]
    fn slh_dsa_sha2_128s_signer_and_verifier_roundtrip() {
        use slh_dsa::signature::Keypair as _;
        let mut rng = rand::rng();
        let signing_key = slh_dsa::SigningKey::<slh_dsa::Sha2_128s>::new(&mut rng);
        let verifying_key = signing_key.verifying_key();
        let key_set = KeySetId([0x44; 32]);
        let purpose = SignaturePurpose::TenantCommit;
        let signer = SlhDsaSha2_128sSigner::new(signing_key, key_set, purpose);
        let verifier = SlhDsaSha2_128sVerifier::new(verifying_key, key_set, purpose);
        roundtrip(
            &signer,
            &verifier,
            &key_set,
            purpose,
            "statechronicle.commit.v0",
            7856,
        );
    }
}

/// AWS-LC's deployed ML-DSA implementation, as an alternative to RustCrypto.
pub struct AwsLcMlDsa65Signer {
    key: aws_lc_rs::signature::PqdsaKeyPair,
    key_set_id: KeySetId,
    purpose: SignaturePurpose,
}
impl AwsLcMlDsa65Signer {
    pub fn new(
        key: aws_lc_rs::signature::PqdsaKeyPair,
        key_set_id: KeySetId,
        purpose: SignaturePurpose,
    ) -> Self {
        Self {
            key,
            key_set_id,
            purpose,
        }
    }
}
impl Signer for AwsLcMlDsa65Signer {
    fn suite(&self) -> &SignatureSuiteId {
        static SUITE: std::sync::OnceLock<SignatureSuiteId> = std::sync::OnceLock::new();
        SUITE.get_or_init(|| SignatureSuiteId::new("ml-dsa-65").expect("static suite is valid"))
    }
    fn sign_into(
        &self,
        request: SigningRequest<'_>,
        output: &mut [u8],
    ) -> Result<usize, SignatureError> {
        check_binding(request, self.suite(), self.key_set_id, self.purpose)?;
        if output.len() < 3309 {
            return Err(SignatureError::CapacityExceeded);
        }
        self.key
            .sign(
                request.transcript,
                output
                    .get_mut(..3309)
                    .ok_or(SignatureError::CapacityExceeded)?,
            )
            .map_err(|_| SignatureError::ProviderUnavailable)
    }
}
pub struct AwsLcMlDsa65Verifier {
    public_key: Vec<u8>,
    key_set_id: KeySetId,
    purpose: SignaturePurpose,
}
impl AwsLcMlDsa65Verifier {
    pub fn new(public_key: Vec<u8>, key_set_id: KeySetId, purpose: SignaturePurpose) -> Self {
        Self {
            public_key,
            key_set_id,
            purpose,
        }
    }
}
impl Verifier for AwsLcMlDsa65Verifier {
    fn suite(&self) -> &SignatureSuiteId {
        static SUITE: std::sync::OnceLock<SignatureSuiteId> = std::sync::OnceLock::new();
        SUITE.get_or_init(|| SignatureSuiteId::new("ml-dsa-65").expect("static suite is valid"))
    }
    fn verify(&self, request: VerificationRequest<'_>) -> Result<(), SignatureError> {
        check_binding(request.request, self.suite(), self.key_set_id, self.purpose)?;
        if request.signature.len() != 3309 {
            return Err(SignatureError::MalformedEncoding);
        }
        aws_lc_rs::signature::UnparsedPublicKey::new(
            &aws_lc_rs::signature::ML_DSA_65,
            &self.public_key,
        )
        .verify(request.request.transcript, request.signature)
        .map_err(|_| SignatureError::InvalidSignature)
    }
}

#[cfg(test)]
mod aws_lc_tests {
    use super::*;
    use aws_lc_rs::signature::{KeyPair, ML_DSA_65_SIGNING, PqdsaKeyPair};
    use statechronicle_core::signature_transcript::signature_transcript;

    #[test]
    fn aws_lc_ml_dsa_65_signer_and_verifier_roundtrip() {
        let signing_key = PqdsaKeyPair::generate(&ML_DSA_65_SIGNING).unwrap();
        let public_key = signing_key.public_key().as_ref().to_vec();
        let key_set = KeySetId([0x55; 32]);
        let purpose = SignaturePurpose::TenantCommit;
        let signer = AwsLcMlDsa65Signer::new(signing_key, key_set, purpose);
        let verifier = AwsLcMlDsa65Verifier::new(public_key, key_set, purpose);
        let suite = signer.suite();
        let transcript = signature_transcript(
            purpose,
            "statechronicle.commit.v0",
            suite,
            &key_set,
            b"AWS-LC transcript",
        )
        .unwrap();
        let request = SigningRequest {
            suite,
            key_set_id: &key_set,
            purpose,
            transcript: &transcript,
        };
        let mut signature = vec![0; 3309];
        assert_eq!(signer.sign_into(request, &mut signature).unwrap(), 3309);
        verifier
            .verify(VerificationRequest {
                request,
                signature: &signature,
            })
            .unwrap();
        let encoded: [u8; 1952] = verifier.public_key.as_slice().try_into().unwrap();
        let rust_crypto_key = ml_dsa::VerifyingKey::<ml_dsa::MlDsa65>::decode(&encoded.into());
        MlDsa65Verifier::new(rust_crypto_key, key_set, purpose)
            .verify(VerificationRequest {
                request,
                signature: &signature,
            })
            .unwrap();
        let wrong_key_set = KeySetId([0x56; 32]);
        let bad_request = SigningRequest {
            suite,
            key_set_id: &wrong_key_set,
            purpose,
            transcript: &transcript,
        };
        assert_eq!(
            signer.sign_into(bad_request, &mut signature),
            Err(SignatureError::KeyUnavailable)
        );
    }
}

#[test]
fn aws_lc_ml_dsa_and_strict_ed25519_are_both_required_by_hybrid() {
    use std::sync::Arc;

    use aws_lc_rs::signature::{KeyPair, ML_DSA_65_SIGNING, PqdsaKeyPair};
    use statechronicle_core::{
        signature_hybrid::{
            HYBRID_SIGNATURE_BYTES, HybridSignature, HybridSigner, HybridSignerMember,
            HybridVerifier, HybridVerifierMember,
        },
        signature_operation::SignatureContext,
        signature_provider::{Ed25519Signer, Ed25519Verifier},
        signature_wire::decode_v1_envelope,
    };

    let purpose = SignaturePurpose::TenantCommit;
    let hybrid_key = KeySetId([1; 32]);
    let ed_key = KeySetId([2; 32]);
    let pq_key = KeySetId([3; 32]);

    let ed_private = ed25519_dalek::SigningKey::from_bytes(&[7; 32]);
    let ed_signer = Ed25519Signer::new(&ed_private, ed_key, purpose);
    let ed_verifier = Ed25519Verifier::new(&ed_private.verifying_key(), ed_key, purpose);
    let pq_private = PqdsaKeyPair::generate(&ML_DSA_65_SIGNING).unwrap();
    let pq_public = pq_private.public_key().as_ref().to_vec();
    let pq_signer = AwsLcMlDsa65Signer::new(pq_private, pq_key, purpose);
    let pq_verifier = AwsLcMlDsa65Verifier::new(pq_public, pq_key, purpose);

    let signer = HybridSigner::new(
        hybrid_key,
        purpose,
        HybridSignerMember {
            signer: &ed_signer,
            key_set_id: ed_key,
        },
        HybridSignerMember {
            signer: &pq_signer,
            key_set_id: pq_key,
        },
    )
    .unwrap();
    let verifier = HybridVerifier::new(
        hybrid_key,
        purpose,
        HybridVerifierMember {
            verifier: Arc::new(ed_verifier),
            key_set_id: ed_key,
        },
        HybridVerifierMember {
            verifier: Arc::new(pq_verifier),
            key_set_id: pq_key,
        },
    )
    .unwrap();
    let context = SignatureContext {
        suite: signer.suite(),
        key_set_id: &hybrid_key,
        purpose,
        body_schema: "statechronicle.commit.v0",
        signature_len: HYBRID_SIGNATURE_BYTES,
    };
    let record = context.sign_envelope(b"layout test body", &signer).unwrap();
    let envelope = decode_v1_envelope(&record, purpose).unwrap();
    context.verify_envelope(envelope, &verifier).unwrap();

    let parts = HybridSignature::decode(envelope.signature).unwrap();
    let mut altered_ed = *parts.ed25519;
    altered_ed[0] ^= 1;
    let mut altered_pq = *parts.ml_dsa_65;
    altered_pq[0] ^= 1;
    for altered in [
        HybridSignature {
            ed25519: &altered_ed,
            ..parts
        },
        HybridSignature {
            ml_dsa_65: &altered_pq,
            ..parts
        },
    ] {
        let mut payload = [0; HYBRID_SIGNATURE_BYTES];
        altered.encode_into(&mut payload).unwrap();
        let mut attempted = envelope;
        attempted.signature = &payload;
        assert_eq!(
            context.verify_envelope(attempted, &verifier),
            Err(SignatureError::InvalidSignature)
        );
    }
}

#[test]
fn nist_acvp_pure_empty_context_vectors() {
    macro_rules! vector {
        ($name:literal) => {
            (
                include_bytes!(concat!("fixtures/acvp/", $name, ".msg")).as_slice(),
                include_bytes!(concat!("fixtures/acvp/", $name, ".pk")).as_slice(),
                include_bytes!(concat!("fixtures/acvp/", $name, ".sig")).as_slice(),
            )
        };
    }

    fn verify(
        verifier: &dyn Verifier,
        key_set_id: &KeySetId,
        (message, _, signature): (&[u8], &[u8], &[u8]),
        valid: bool,
    ) {
        let result = verifier.verify(VerificationRequest {
            request: SigningRequest {
                suite: verifier.suite(),
                key_set_id,
                purpose: SignaturePurpose::TenantCommit,
                transcript: message,
            },
            signature,
        });
        assert_eq!(result.is_ok(), valid, "{result:?}");
    }

    let key_set = KeySetId([23; 32]);
    let purpose = SignaturePurpose::TenantCommit;
    for (sample, valid) in [
        (vector!("ml-dsa-65-positive"), true),
        (vector!("ml-dsa-65-negative"), false),
    ] {
        let encoded: [u8; 1952] = sample.1.try_into().unwrap();
        let key = ml_dsa::VerifyingKey::<ml_dsa::MlDsa65>::decode(&encoded.into());
        verify(
            &MlDsa65Verifier::new(key, key_set, purpose),
            &key_set,
            sample,
            valid,
        );
        verify(
            &AwsLcMlDsa65Verifier::new(sample.1.to_vec(), key_set, purpose),
            &key_set,
            sample,
            valid,
        );
    }
    for (sample, valid) in [
        (vector!("slh-dsa-sha2-128s-positive"), true),
        (vector!("slh-dsa-sha2-128s-negative"), false),
    ] {
        let key = slh_dsa::VerifyingKey::<slh_dsa::Sha2_128s>::try_from(sample.1).unwrap();
        verify(
            &SlhDsaSha2_128sVerifier::new(key, key_set, purpose),
            &key_set,
            sample,
            valid,
        );
    }
}
