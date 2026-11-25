//! Cryptographic processing after the host resolves trusted signature policy.

use crate::signature::SIGNATURE_BYTE_LEN;
use crate::signature_hybrid::{
    HYBRID_SIGNATURE_BYTES, HYBRID_SUITE_ID, HybridSignature, ML_DSA_65_SIGNATURE_BYTES,
};
use crate::signature_provider::{
    KeySetId, MAX_SIGNATURE_BYTES, SignatureError, SignaturePurpose, SignatureSuiteId, Signer,
    SigningRequest, VerificationRequest, Verifier,
};
use crate::signature_transcript::signature_transcript;
use crate::signature_wire::{V1SignedEnvelope, encode_v1_envelope};

/// Suite and key-set binding selected by the host for a typed protocol body.
pub struct SignatureContext<'policy> {
    /// Suite required by policy.
    pub suite: &'policy SignatureSuiteId,
    /// Key set required by policy.
    pub key_set_id: &'policy KeySetId,
    /// Purpose selected by the typed protocol caller.
    pub purpose: SignaturePurpose,
    /// Schema expected by the typed protocol caller.
    pub body_schema: &'policy str,
    /// Exact signature size declared by the registered suite.
    pub signature_len: usize,
}

impl SignatureContext<'_> {
    fn check_provider(&self, suite: &SignatureSuiteId) -> Result<(), SignatureError> {
        if self.suite != suite {
            return Err(SignatureError::UnsupportedSuite);
        }
        if self.signature_len == 0 || self.signature_len > MAX_SIGNATURE_BYTES {
            return Err(SignatureError::CapacityExceeded);
        }
        let expected = match suite.as_str() {
            "ed25519" => SIGNATURE_BYTE_LEN,
            "ml-dsa-65" => ML_DSA_65_SIGNATURE_BYTES,
            "slh-dsa-sha2-128s" => 7856,
            HYBRID_SUITE_ID => HYBRID_SIGNATURE_BYTES,
            _ => self.signature_len,
        };
        if expected != self.signature_len {
            return Err(SignatureError::MalformedEncoding);
        }
        Ok(())
    }

    fn check_payload(&self, signature: &[u8]) -> Result<(), SignatureError> {
        if signature.len() != self.signature_len {
            return Err(SignatureError::MalformedEncoding);
        }
        if self.suite.as_str() == HYBRID_SUITE_ID {
            HybridSignature::decode(signature)?;
        }
        Ok(())
    }

    fn transcript(&self, body: &[u8]) -> Result<Vec<u8>, SignatureError> {
        signature_transcript(
            self.purpose,
            self.body_schema,
            self.suite,
            self.key_set_id,
            body,
        )
    }

    const fn request<'input>(&'input self, transcript: &'input [u8]) -> SigningRequest<'input> {
        SigningRequest {
            suite: self.suite,
            key_set_id: self.key_set_id,
            purpose: self.purpose,
            transcript,
        }
    }

    /// Signs a canonical body and encodes its standalone v1 envelope.
    ///
    /// # Errors
    /// Rejects mismatched providers, invalid lengths, and provider failures.
    /// A partially signed payload is discarded.
    pub fn sign_envelope(
        &self,
        body: &[u8],
        signer: &dyn Signer,
    ) -> Result<Vec<u8>, SignatureError> {
        self.check_provider(signer.suite())?;
        let transcript = self.transcript(body)?;
        let mut signature = vec![0; self.signature_len];
        let written = signer.sign_into(self.request(&transcript), &mut signature)?;
        if written != signature.len() {
            return Err(SignatureError::MalformedEncoding);
        }
        self.check_payload(&signature)?;
        encode_v1_envelope(
            V1SignedEnvelope {
                body,
                suite_id: self.suite.as_str(),
                key_set_id: &self.key_set_id.0,
                signature: &signature,
            },
            self.purpose,
        )
    }

    /// Verifies the envelope signature using the independently selected context.
    ///
    /// # Errors
    /// Rejects a mismatched envelope binding or malformed payload before crypto.
    /// Provider errors are returned directly.
    pub fn verify_envelope(
        &self,
        envelope: V1SignedEnvelope<'_>,
        verifier: &dyn Verifier,
    ) -> Result<(), SignatureError> {
        if self.suite.as_str() != envelope.suite_id || self.key_set_id.0 != *envelope.key_set_id {
            return Err(SignatureError::UnauthorizedSuite);
        }
        self.check_provider(verifier.suite())?;
        self.check_payload(envelope.signature)?;
        let transcript = self.transcript(envelope.body)?;
        verifier.verify(VerificationRequest {
            request: self.request(&transcript),
            signature: envelope.signature,
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use ed25519_dalek::SigningKey;

    use crate::signature_provider::{Ed25519Signer, Ed25519Verifier};
    use crate::signature_wire::decode_v1_envelope;

    use super::*;

    #[test]
    fn context_binds_payload_to_purpose_schema_suite_and_key() {
        let key = SigningKey::from_bytes(&[42; 32]);
        let verifying_key = key.verifying_key();
        let key_set_id = KeySetId([7; 32]);
        let purpose = SignaturePurpose::TenantCommit;
        let signer = Ed25519Signer::new(&key, key_set_id, purpose);
        let verifier = Ed25519Verifier::new(&verifying_key, key_set_id, purpose);
        let suite = signer.suite();
        let context = SignatureContext {
            suite,
            key_set_id: &key_set_id,
            purpose,
            body_schema: "statechronicle.commit.v0",
            signature_len: SIGNATURE_BYTE_LEN,
        };
        let bytes = context.sign_envelope(b"example body", &signer).unwrap();
        let envelope = decode_v1_envelope(&bytes, purpose).unwrap();
        context.verify_envelope(envelope, &verifier).unwrap();

        for wrong_binding in [
            V1SignedEnvelope {
                key_set_id: &[8; 32],
                ..envelope
            },
            V1SignedEnvelope {
                suite_id: "ml-dsa-65",
                ..envelope
            },
        ] {
            assert_eq!(
                context.verify_envelope(wrong_binding, &verifier),
                Err(SignatureError::UnauthorizedSuite)
            );
        }
        assert_eq!(
            context.verify_envelope(
                V1SignedEnvelope {
                    signature: &[0; 63],
                    ..envelope
                },
                &verifier,
            ),
            Err(SignatureError::MalformedEncoding)
        );
        let other_context = SignatureContext {
            body_schema: "statechronicle.commit.v1",
            ..context
        };
        assert_eq!(
            other_context.verify_envelope(envelope, &verifier),
            Err(SignatureError::InvalidSignature)
        );
    }
}
