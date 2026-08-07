//! SHA-256 content digests in canonical `sha256:<lowercase-hex>` form.
//!
//! Digests are computed over BCS canonical bytes (ADR-004) so that the same
//! logical object always yields the same digest, independent of key order or
//! formatting. [`ContentDigest`] is the protocol's validated digest newtype
//! (§17): the only string→digest boundary is
//! [`ContentDigest::from_hex_sha256`] / [`FromStr`], and everything else is
//! typed.

use core::fmt;
use core::str::FromStr;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest as _, Sha256};

use crate::error::StateChronicleError;

/// The `sha256:` prefix that every `ContentDigest` string form starts with.
pub const DIGEST_PREFIX: &str = "sha256:";

/// The number of raw digest bytes (SHA-256 output length).
pub const DIGEST_BYTE_LEN: usize = 32;

/// The number of lowercase hex characters encoding the digest bytes.
pub const DIGEST_HEX_LEN: usize = 64;

/// A validated SHA-256 content digest in canonical `sha256:<lowercase-hex>`
/// form.
///
/// The digest is stored as both its 32 raw bytes and its canonical string so
/// that [`as_bytes`](Self::as_bytes) and [`as_str`](Self::as_str) return
/// borrowed views without re-encoding. Values are validated at construction;
/// callers never hold a raw string that invites comparison or dispatch.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ContentDigest {
    bytes: [u8; DIGEST_BYTE_LEN],
    encoded: String,
}

impl ContentDigest {
    /// Constructs a `ContentDigest` from its 32 raw SHA-256 bytes.
    ///
    /// The canonical `sha256:<lowercase-hex>` string form is derived from
    /// `bytes`, so the same bytes always produce the same digest.
    pub fn new(bytes: [u8; DIGEST_BYTE_LEN]) -> Self {
        let hex_encoded = hex::encode(bytes);
        let encoded = format!("{DIGEST_PREFIX}{hex_encoded}");
        Self { bytes, encoded }
    }

    /// Parses and validates a `sha256:<lowercase-hex>` string.
    ///
    /// This is the only raw-string boundary for digests: everything else
    /// operates on the typed newtype.
    ///
    /// # Errors
    ///
    /// Returns [`StateChronicleError::InvalidDigest`] when `s` does not start
    /// with the `sha256:` prefix, is not exactly 64 lowercase hex characters,
    /// or fails to decode to 32 bytes.
    pub fn from_hex_sha256(s: &str) -> Result<Self, StateChronicleError> {
        let hex_part = s.strip_prefix(DIGEST_PREFIX).ok_or_else(|| {
            StateChronicleError::InvalidDigest(format!(
                "digest must start with `{DIGEST_PREFIX}`, got `{s}`"
            ))
        })?;
        if hex_part.len() != DIGEST_HEX_LEN {
            return Err(StateChronicleError::InvalidDigest(format!(
                "digest hex must be exactly {DIGEST_HEX_LEN} characters, got {}",
                hex_part.len()
            )));
        }
        if !hex_part.bytes().all(is_lower_hex_byte) {
            return Err(StateChronicleError::InvalidDigest(format!(
                "digest hex must be lowercase hexadecimal, got `{hex_part}`"
            )));
        }
        let decoded = hex::decode(hex_part).map_err(|source| {
            StateChronicleError::InvalidDigest(format!(
                "digest hex is not valid hexadecimal: {source}"
            ))
        })?;
        let bytes: [u8; DIGEST_BYTE_LEN] = decoded.try_into().map_err(|decoded: Vec<u8>| {
            StateChronicleError::InvalidDigest(format!(
                "digest must decode to {DIGEST_BYTE_LEN} bytes, got {}",
                decoded.len()
            ))
        })?;
        Ok(Self::new(bytes))
    }

    /// Returns the canonical `sha256:<lowercase-hex>` string form.
    pub fn as_str(&self) -> &str {
        &self.encoded
    }

    /// Returns the 32 raw digest bytes.
    pub const fn as_bytes(&self) -> &[u8; DIGEST_BYTE_LEN] {
        &self.bytes
    }
}

/// Computes the protocol digest of `bytes`: `sha256:<lowercase-hex>`.
///
/// This is the protocol's `digest()` (§17 / ADR-004): SHA-256 over the
/// canonical BCS bytes of a protocol object. Deterministic: the same input
/// always produces the same digest, and the function is total. It never
/// fails or panics.
pub fn hash_bytes(bytes: &[u8]) -> ContentDigest {
    ContentDigest::new(Sha256::digest(bytes).into())
}

impl fmt::Display for ContentDigest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ContentDigest {
    type Err = StateChronicleError;

    /// Parses a `sha256:<lowercase-hex>` string.
    ///
    /// # Errors
    ///
    /// Returns [`StateChronicleError::InvalidDigest`] for any input that is
    /// not a valid canonical digest string.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_hex_sha256(s)
    }
}

impl Serialize for ContentDigest {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ContentDigest {
    /// Deserializes from the canonical `sha256:<lowercase-hex>` string form.
    ///
    /// # Errors
    ///
    /// Returns an error when the deserialized string is not a valid canonical
    /// digest.
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Self::from_hex_sha256(&value).map_err(D::Error::custom)
    }
}

/// Returns whether `byte` is an ASCII lowercase hexadecimal digit.
const fn is_lower_hex_byte(byte: u8) -> bool {
    matches!(byte, b'0'..=b'9' | b'a'..=b'f')
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use std::str::FromStr;

    use super::{ContentDigest, DIGEST_BYTE_LEN, DIGEST_HEX_LEN, DIGEST_PREFIX, hash_bytes};
    use crate::error::StateChronicleError;

    #[test]
    fn new_roundtrips_through_string_form() {
        let digest = ContentDigest::new([0u8; DIGEST_BYTE_LEN]);
        assert!(digest.as_str().starts_with(DIGEST_PREFIX));

        let reparsed = ContentDigest::from_hex_sha256(digest.as_str()).unwrap();
        assert_eq!(reparsed, digest);
        assert_eq!(reparsed.as_str(), digest.as_str());
    }

    #[test]
    fn as_bytes_returns_raw_digest_bytes() {
        let bytes = [0xabu8; DIGEST_BYTE_LEN];
        let digest = ContentDigest::new(bytes);
        assert_eq!(digest.as_bytes(), &bytes);
    }

    #[test]
    fn from_str_and_display_roundtrip() {
        let digest = hash_bytes(b"from-str");
        let parsed = ContentDigest::from_str(digest.as_str()).unwrap();
        assert_eq!(parsed, digest);
        assert_eq!(parsed.to_string(), digest.as_str());
    }

    #[test]
    fn from_hex_sha256_accepts_valid_lowercase_digest() {
        // 16 hex characters repeated 4 times = exactly 64 lowercase hex digits.
        let valid = format!("{DIGEST_PREFIX}{}", "0123456789abcdef".repeat(4));
        let digest = ContentDigest::from_hex_sha256(&valid).unwrap();
        assert_eq!(digest.as_str(), valid);
    }

    #[test]
    fn from_hex_sha256_rejects_wrong_prefix() {
        let raw = format!("md5:{}", "0".repeat(DIGEST_HEX_LEN));
        assert!(matches!(
            ContentDigest::from_hex_sha256(&raw),
            Err(StateChronicleError::InvalidDigest(_))
        ));
    }

    #[test]
    fn from_hex_sha256_rejects_missing_prefix() {
        let raw = "0".repeat(DIGEST_HEX_LEN);
        assert!(ContentDigest::from_hex_sha256(&raw).is_err());
    }

    #[test]
    fn from_hex_sha256_rejects_uppercase_hex() {
        let raw = format!("{DIGEST_PREFIX}{}", "A".repeat(DIGEST_HEX_LEN));
        assert!(ContentDigest::from_hex_sha256(&raw).is_err());
    }

    #[test]
    fn from_hex_sha256_rejects_non_hex_characters() {
        let raw = format!("{DIGEST_PREFIX}{}", "z".repeat(DIGEST_HEX_LEN));
        assert!(ContentDigest::from_hex_sha256(&raw).is_err());
    }

    #[test]
    fn from_hex_sha256_rejects_too_short() {
        let raw = format!(
            "{DIGEST_PREFIX}{}",
            "a".repeat(DIGEST_HEX_LEN.saturating_sub(1))
        );
        assert!(ContentDigest::from_hex_sha256(&raw).is_err());
    }

    #[test]
    fn from_hex_sha256_rejects_too_long() {
        let raw = format!(
            "{DIGEST_PREFIX}{}",
            "a".repeat(DIGEST_HEX_LEN.saturating_add(1))
        );
        assert!(ContentDigest::from_hex_sha256(&raw).is_err());
    }

    #[test]
    fn from_hex_sha256_rejects_empty_string() {
        assert!(ContentDigest::from_hex_sha256("").is_err());
    }

    #[test]
    fn hash_bytes_known_answer_empty_string() {
        // sha256 of the empty string, the standard first known-answer vector.
        let digest = hash_bytes(b"");
        assert_eq!(
            digest.as_str(),
            "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn hash_bytes_known_answer_abc() {
        // sha256("abc"), the canonical second known-answer vector.
        let digest = hash_bytes(b"abc");
        assert_eq!(
            digest.as_str(),
            "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn hash_bytes_is_deterministic() {
        let input = b"same input twice";
        assert_eq!(hash_bytes(input), hash_bytes(input));
    }

    #[test]
    fn hash_bytes_output_decodes_to_32_bytes() {
        let digest = hash_bytes(b"decode-check");
        assert_eq!(digest.as_bytes().len(), DIGEST_BYTE_LEN);
    }

    #[test]
    fn parse_then_as_bytes_preserves_bytes() {
        let digest = hash_bytes(b"bytes-preserved");
        let parsed = ContentDigest::from_hex_sha256(digest.as_str()).unwrap();
        assert_eq!(parsed.as_bytes(), digest.as_bytes());
    }

    #[test]
    fn serde_roundtrips_through_string_form() {
        let digest = hash_bytes(b"serde");
        let json = serde_json::to_string(&digest).unwrap();
        assert!(json.starts_with("\"sha256:"));

        let decoded: ContentDigest = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, digest);
    }

    #[test]
    fn deserialize_rejects_invalid_string() {
        let result = serde_json::from_str::<ContentDigest>("\"sha256:zz\"");
        assert!(result.is_err());
    }
}
