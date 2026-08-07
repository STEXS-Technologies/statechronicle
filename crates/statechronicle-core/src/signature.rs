//! Ed25519 signatures over canonicalized content (ADR-004 §5).
//!
//! Commit and snapshot keys are Ed25519, rotated through TrustGrant-authorized
//! procedures. Signatures bind the BCS canonical bytes of a protocol body to a
//! signing key; per the structural envelope rule (ADR-004 §2) a signature
//! never covers a `signature` field. The serde/Display string form follows the
//! protocol's `b64u:` base64url convention (§17), matching the `b64u:` nonce
//! and signature strings used by the intent envelope.

use core::fmt;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ed25519_dalek::Signature as DalekSignature;
use ed25519_dalek::Signer as _;
use ed25519_dalek::{SigningKey, VerifyingKey};
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::error::StateChronicleError;

/// The number of bytes in an Ed25519 signature.
pub const SIGNATURE_BYTE_LEN: usize = 64;

/// The `b64u:` prefix used by the protocol's signature string form (§17).
///
/// Encoding is RFC 4648 base64url **without padding**, as implied by the
/// protocol's `b64u:` convention for nonces and signatures.
pub const B64U_PREFIX: &str = "b64u:";

/// An Ed25519 signature over BCS canonical bytes.
///
/// Stored as the raw 64 signature bytes. Serde serialization and [`Display`]
/// use the protocol's `b64u:<base64url-unpadded>` string form (§17); the raw
/// bytes are exposed via [`as_bytes`](Self::as_bytes) and construction from
/// raw bytes via [`from_bytes`](Self::from_bytes).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Signature {
    bytes: [u8; SIGNATURE_BYTE_LEN],
}

impl Signature {
    /// Constructs a `Signature` from its raw 64 bytes.
    ///
    /// Any 64 bytes form a valid Ed25519 `Signature` value; validity against a
    /// specific message and key is established by [`verify`].
    pub const fn from_bytes(bytes: [u8; SIGNATURE_BYTE_LEN]) -> Self {
        Self { bytes }
    }

    /// Returns the raw 64 signature bytes.
    pub const fn as_bytes(&self) -> &[u8; SIGNATURE_BYTE_LEN] {
        &self.bytes
    }

    /// Encodes the signature in the protocol's `b64u:` string form (§17).
    fn b64u_string(self) -> String {
        format!("{B64U_PREFIX}{}", URL_SAFE_NO_PAD.encode(self.bytes))
    }
}

/// Signs the canonical bytes with the given Ed25519 signing key.
///
/// Per ADR-004 the signature covers the BCS canonical representation of the
/// signed body — never a `signature` field (structural envelope, §2).
pub fn sign(canonical: &[u8], key: &SigningKey) -> Signature {
    let signature = key.sign(canonical);
    Signature::from_bytes(signature.to_bytes())
}

/// Verifies `sig` over `canonical` against the given Ed25519 verifying key.
///
/// Uses Ed25519 strict verification (ZIP-215 malleability checks): the
/// signature, the public key, and the scalar are all checked, so a
/// malformed, weak-key, or malleable signature is rejected.
///
/// # Errors
///
/// Returns [`StateChronicleError::SignatureVerification`] when the signature
/// does not match `canonical` under `key`, or fails any strict-verification
/// check.
pub fn verify(
    canonical: &[u8],
    key: &VerifyingKey,
    sig: &Signature,
) -> Result<(), StateChronicleError> {
    let dalek_signature = DalekSignature::from_bytes(sig.as_bytes());
    key.verify_strict(canonical, &dalek_signature)
        .map_err(|source| {
            StateChronicleError::SignatureVerification(format!(
                "Ed25519 strict verification failed: {source}"
            ))
        })
}

impl fmt::Display for Signature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.b64u_string())
    }
}

impl Serialize for Signature {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.b64u_string())
    }
}

impl<'de> Deserialize<'de> for Signature {
    /// Deserializes from the protocol's `b64u:<base64url-unpadded>` string
    /// form.
    ///
    /// # Errors
    ///
    /// Returns an error when the string does not start with `b64u:`, is not
    /// valid base64url, or does not decode to exactly 64 bytes.
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        let encoded = value.strip_prefix(B64U_PREFIX).ok_or_else(|| {
            D::Error::custom(format!("signature must start with `{B64U_PREFIX}`"))
        })?;
        let decoded = URL_SAFE_NO_PAD.decode(encoded).map_err(|source| {
            D::Error::custom(format!("signature is not valid base64url: {source}"))
        })?;
        let bytes: [u8; SIGNATURE_BYTE_LEN] = decoded.try_into().map_err(|decoded: Vec<u8>| {
            D::Error::custom(format!(
                "signature must be exactly {SIGNATURE_BYTE_LEN} bytes, got {}",
                decoded.len()
            ))
        })?;
        Ok(Self::from_bytes(bytes))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use ed25519_dalek::{SigningKey, VerifyingKey};

    use super::{B64U_PREFIX, Signature, sign, verify};

    const FIXED_SEED: [u8; 32] = [42u8; 32];
    const FIXED_MESSAGE: &[u8] = b"statechronicle known-answer message\n";

    fn fixed_key() -> SigningKey {
        SigningKey::from_bytes(&FIXED_SEED)
    }

    #[test]
    fn sign_then_verify_succeeds() {
        let key = fixed_key();
        let canonical = b"canonical bytes to sign";

        let signature = sign(canonical, &key);
        assert!(verify(canonical, &key.verifying_key(), &signature).is_ok());
    }

    #[test]
    fn verify_rejects_tampered_canonical() {
        let key = fixed_key();
        let canonical = b"canonical bytes to sign";
        let signature = sign(canonical, &key);

        let mut tampered = canonical.to_vec();
        tampered.push(0x00);

        assert!(verify(&tampered, &key.verifying_key(), &signature).is_err());
    }

    #[test]
    fn verify_rejects_wrong_key() {
        let key = fixed_key();
        let other_key = SigningKey::from_bytes(&[7u8; 32]);
        let canonical = b"canonical bytes to sign";

        let signature = sign(canonical, &key);
        assert!(verify(canonical, &other_key.verifying_key(), &signature).is_err());
    }

    #[test]
    fn known_answer_signature_is_stable() {
        let key = fixed_key();
        let signature = sign(FIXED_MESSAGE, &key);
        let again = sign(FIXED_MESSAGE, &key);

        // A fixed seed + fixed message must always produce the same bytes.
        assert_eq!(signature.as_bytes(), again.as_bytes());
        assert_eq!(
            hex::encode(signature.as_bytes()),
            "624d6d63f1d8247292aa39b00098d9ce71a5251a988ae163dcc518e2adaecce9f247a069519041b68f02f1e0a4a0a50951dea10d79a9713b0d23b640e6ebb40b"
        );
    }

    #[test]
    fn display_uses_b64u_string_form() {
        let key = fixed_key();
        let signature = sign(b"display check", &key);

        let rendered = signature.to_string();
        assert!(rendered.starts_with(B64U_PREFIX));
        assert!(!rendered.starts_with(&format!("{B64U_PREFIX}{B64U_PREFIX}")));
    }

    #[test]
    fn serde_roundtrips_through_b64u_string_form() {
        let key = fixed_key();
        let signature = sign(b"serde check", &key);

        let json = serde_json::to_string(&signature).unwrap();
        assert!(json.starts_with("\"b64u:"));

        let decoded: Signature = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, signature);
    }

    #[test]
    fn deserialize_rejects_invalid_b64u_strings() {
        let not_prefixed = serde_json::from_str::<Signature>("\"QUJD\"");
        assert!(not_prefixed.is_err());

        let not_base64 = serde_json::from_str::<Signature>("\"b64u:%%%\"");
        assert!(not_base64.is_err());

        let wrong_length = serde_json::from_str::<Signature>("\"b64u:QUJD\"");
        assert!(wrong_length.is_err());
    }

    #[test]
    fn from_bytes_roundtrips_raw_bytes() {
        let key = fixed_key();
        let signature = sign(b"bytes roundtrip", &key);
        let bytes = *signature.as_bytes();

        let reconstructed = Signature::from_bytes(bytes);
        assert_eq!(reconstructed, signature);
    }

    #[test]
    fn signing_key_exported_to_verifying_key() {
        let key = fixed_key();
        let verifying: VerifyingKey = key.verifying_key();

        assert!(verify(b"any message", &verifying, &sign(b"any message", &key)).is_ok());
    }
}
