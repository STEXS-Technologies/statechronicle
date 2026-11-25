//! Borrowed cryptographic provider contracts for versioned signatures.
//!
//! Protocol entry points select trusted policy and build transcripts before
//! invoking these primitives. Provider success alone conveys no authority.

use std::{borrow::Cow, future::Future, pin::Pin};

use ed25519_dalek::{SigningKey, VerifyingKey};

use crate::signature::{self, SIGNATURE_BYTE_LEN, Signature};

/// Upper limit on a v1 signature payload, in bytes.
pub const MAX_SIGNATURE_BYTES: usize = 16 * 1024;

/// A syntactically valid suite identifier. Policy determines whether it is enabled.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SignatureSuiteId(Cow<'static, str>);

pub(crate) fn validate_suite_id(id: &str) -> Result<(), SignatureError> {
    let Some((first, rest)) = id.as_bytes().split_first() else {
        return Err(SignatureError::MalformedEncoding);
    };
    if id.len() > 64
        || !first.is_ascii_lowercase()
        || !rest.iter().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'.')
        })
    {
        return Err(SignatureError::MalformedEncoding);
    }
    Ok(())
}

impl SignatureSuiteId {
    /// Validates the syntax of a suite identifier.
    ///
    /// # Errors
    /// Returns an encoding error for identifiers outside the permitted ASCII grammar.
    pub fn new(id: &str) -> Result<Self, SignatureError> {
        validate_suite_id(id)?;
        Ok(Self(Cow::Owned(id.to_owned())))
    }

    /// Returns the canonical ASCII suite identifier.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Registry-assigned identity of a single immutable key set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct KeySetId(pub [u8; 32]);

/// The protocol operation for which the transcript was constructed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum SignaturePurpose {
    /// An actor's request to perform an operation.
    ActorIntent = 1,
    /// A tenant authority's commit.
    TenantCommit = 2,
    /// A global authority's checkpoint.
    GlobalCheckpoint = 3,
}

/// Inputs shared by signing and verification providers.
#[derive(Debug, Clone, Copy)]
pub struct SigningRequest<'input> {
    /// Suite selected by trusted policy.
    pub suite: &'input SignatureSuiteId,
    /// Registry identity selected by trusted policy.
    pub key_set_id: &'input KeySetId,
    /// Purpose selected by the protocol entry point.
    pub purpose: SignaturePurpose,
    /// Complete library-constructed transcript.
    pub transcript: &'input [u8],
}

/// Inputs for verification.
#[derive(Debug, Clone, Copy)]
pub struct VerificationRequest<'input> {
    /// Trusted context and transcript.
    pub request: SigningRequest<'input>,
    /// Untrusted signature payload.
    pub signature: &'input [u8],
}

/// Failure at the cryptographic provider boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SignatureError {
    /// The signature failed cryptographic verification.
    #[error("invalid signature")]
    InvalidSignature,
    /// The provider does not implement the requested suite.
    #[error("unsupported signature suite")]
    UnsupportedSuite,
    /// Trusted policy does not permit the requested operation.
    #[error("signature policy refused the request")]
    UnauthorizedSuite,
    /// The requested key set is unavailable to the provider.
    #[error("signature key set unavailable")]
    KeyUnavailable,
    /// An identifier or signature payload is malformed.
    #[error("malformed signature encoding")]
    MalformedEncoding,
    /// The signature cannot fit within the supplied capacity.
    #[error("signature capacity exceeded")]
    CapacityExceeded,
    /// The external provider could not complete the request.
    #[error("signature provider unavailable")]
    ProviderUnavailable,
}

/// Object-safe provider for synchronous signing.
pub trait Signer: Send + Sync {
    /// Identifies the provider's suite.
    fn suite(&self) -> &SignatureSuiteId;

    /// Writes a complete signature and returns its exact length.
    ///
    /// # Errors
    /// Returns a provider, binding, or capacity failure. The caller discards
    /// the output buffer on failure.
    fn sign_into(
        &self,
        request: SigningRequest<'_>,
        output: &mut [u8],
    ) -> Result<usize, SignatureError>;
}

/// In-flight signing operation borrowing its request and output buffer.
pub type SigningFuture<'operation> =
    Pin<Box<dyn Future<Output = Result<usize, SignatureError>> + Send + 'operation>>;

/// Object-safe, nonblocking signing provider for external key services.
pub trait AsyncSigner: Send + Sync {
    /// Identifies the provider's suite.
    fn suite(&self) -> &SignatureSuiteId;

    /// Writes a complete signature into the caller's bounded buffer.
    ///
    /// The adapter owns deadlines, cancellation and concurrency limits.
    /// Dropping the future or returning an error discards the output.
    fn sign_into<'operation>(
        &'operation self,
        request: SigningRequest<'operation>,
        output: &'operation mut [u8],
    ) -> SigningFuture<'operation>;
}

/// Object-safe provider for synchronous verification.
pub trait Verifier: Send + Sync {
    /// Identifies the provider's suite.
    fn suite(&self) -> &SignatureSuiteId;

    /// Verifies a signature against an already selected key set.
    ///
    /// # Errors
    /// Returns a binding, encoding, provider, or cryptographic failure.
    fn verify(&self, request: VerificationRequest<'_>) -> Result<(), SignatureError>;
}

pub(crate) struct ProviderBinding {
    pub(crate) suite: SignatureSuiteId,
    key_set_id: KeySetId,
    purpose: SignaturePurpose,
}

impl ProviderBinding {
    pub(crate) const fn new(
        suite: &'static str,
        key_set_id: KeySetId,
        purpose: SignaturePurpose,
    ) -> Self {
        Self {
            suite: SignatureSuiteId(Cow::Borrowed(suite)),
            key_set_id,
            purpose,
        }
    }

    pub(crate) fn check(&self, request: SigningRequest<'_>) -> Result<(), SignatureError> {
        if *request.suite != self.suite {
            return Err(SignatureError::UnsupportedSuite);
        }
        if *request.key_set_id != self.key_set_id {
            return Err(SignatureError::KeyUnavailable);
        }
        if request.purpose != self.purpose {
            return Err(SignatureError::UnauthorizedSuite);
        }
        Ok(())
    }
}

/// Ed25519 signer for a single trusted key set and purpose.
pub struct Ed25519Signer<'key> {
    binding: ProviderBinding,
    key: &'key SigningKey,
}

impl<'key> Ed25519Signer<'key> {
    /// Binds an Ed25519 signing key to the key set and purpose selected by policy.
    pub const fn new(
        key: &'key SigningKey,
        key_set_id: KeySetId,
        purpose: SignaturePurpose,
    ) -> Self {
        Self {
            binding: ProviderBinding::new("ed25519", key_set_id, purpose),
            key,
        }
    }
}

impl Signer for Ed25519Signer<'_> {
    fn suite(&self) -> &SignatureSuiteId {
        &self.binding.suite
    }

    fn sign_into(
        &self,
        request: SigningRequest<'_>,
        output: &mut [u8],
    ) -> Result<usize, SignatureError> {
        self.binding.check(request)?;
        let destination = output
            .get_mut(..SIGNATURE_BYTE_LEN)
            .ok_or(SignatureError::CapacityExceeded)?;
        destination.copy_from_slice(signature::sign(request.transcript, self.key).as_bytes());
        Ok(SIGNATURE_BYTE_LEN)
    }
}

/// Strict Ed25519 verifier for a single trusted key set and purpose.
pub struct Ed25519Verifier {
    binding: ProviderBinding,
    key: VerifyingKey,
}

impl Ed25519Verifier {
    /// Binds an Ed25519 verification key to the key set and purpose selected by policy.
    pub const fn new(key: &VerifyingKey, key_set_id: KeySetId, purpose: SignaturePurpose) -> Self {
        Self {
            binding: ProviderBinding::new("ed25519", key_set_id, purpose),
            key: *key,
        }
    }
}

impl Verifier for Ed25519Verifier {
    fn suite(&self) -> &SignatureSuiteId {
        &self.binding.suite
    }

    fn verify(&self, request: VerificationRequest<'_>) -> Result<(), SignatureError> {
        self.binding.check(request.request)?;
        let bytes = request
            .signature
            .try_into()
            .map_err(|_length_error| SignatureError::MalformedEncoding)?;
        signature::verify(
            request.request.transcript,
            &self.key,
            &Signature::from_bytes(bytes),
        )
        .map_err(|_verification_error| SignatureError::InvalidSignature)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn suite_identifiers_are_ascii_and_bounded() {
        assert!(SignatureSuiteId::new(&"a".repeat(64)).is_ok());
        assert!(SignatureSuiteId::new(&"a".repeat(65)).is_err());
        assert!(SignatureSuiteId::new("ml-dsa-65.v1").is_ok());
        for invalid in ["", ".ed25519", "Ed25519", "x/y", "é"] {
            assert!(SignatureSuiteId::new(invalid).is_err());
        }
    }

    #[test]
    fn provider_bytes_match_legacy_ed25519() {
        let key = SigningKey::from_bytes(&[42; 32]);
        let verifying_key = key.verifying_key();
        let key_set_id = KeySetId([7; 32]);
        let purpose = SignaturePurpose::TenantCommit;
        let signer = Ed25519Signer::new(&key, key_set_id, purpose);
        let verifier = Ed25519Verifier::new(&verifying_key, key_set_id, purpose);
        let signer: &dyn Signer = &signer;
        let verifier: &dyn Verifier = &verifier;
        let request = SigningRequest {
            suite: signer.suite(),
            key_set_id: &key_set_id,
            purpose,
            transcript: b"provider compatibility vector",
        };
        let mut output = [0; SIGNATURE_BYTE_LEN];
        assert_eq!(
            signer.sign_into(request, &mut output).unwrap(),
            output.len()
        );
        assert_eq!(
            &output,
            signature::sign(request.transcript, &key).as_bytes()
        );
        let valid = VerificationRequest {
            request,
            signature: &output,
        };
        verifier.verify(valid).unwrap();
        assert_eq!(
            verifier.verify(VerificationRequest {
                request: SigningRequest {
                    transcript: b"changed transcript",
                    ..request
                },
                ..valid
            }),
            Err(SignatureError::InvalidSignature)
        );

        let mut short = [0x5a; SIGNATURE_BYTE_LEN - 1];
        assert_eq!(
            signer.sign_into(request, &mut short),
            Err(SignatureError::CapacityExceeded)
        );
        assert_eq!(short, [0x5a; SIGNATURE_BYTE_LEN - 1]);
        assert_eq!(
            verifier.verify(VerificationRequest {
                signature: &short,
                ..valid
            }),
            Err(SignatureError::MalformedEncoding)
        );

        let other_suite = SignatureSuiteId::new("ml-dsa-65").unwrap();
        let other_key_set = KeySetId([8; 32]);
        for (bad_request, error) in [
            (
                SigningRequest {
                    suite: &other_suite,
                    ..request
                },
                SignatureError::UnsupportedSuite,
            ),
            (
                SigningRequest {
                    key_set_id: &other_key_set,
                    ..request
                },
                SignatureError::KeyUnavailable,
            ),
            (
                SigningRequest {
                    purpose: SignaturePurpose::ActorIntent,
                    ..request
                },
                SignatureError::UnauthorizedSuite,
            ),
        ] {
            assert_eq!(signer.sign_into(bad_request, &mut short), Err(error));
            assert_eq!(
                verifier.verify(VerificationRequest {
                    request: bad_request,
                    ..valid
                }),
                Err(error)
            );
        }
    }
}
