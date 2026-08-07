//! Intents: requested state transitions (protocol §11).
//!
//! An intent describes the transition a client requests. It is parsed and
//! validated (in `statechronicle-intent`) before execution.
//!
//! Operations are registry-open and profile-defined (e.g. `asset.transfer`,
//! `currency.transfer`, `stack.consume`). The `inputs` map is also
//! profile-defined; it is stored as a `BTreeMap` so BCS canonicalization sorts
//! keys deterministically. Floats in `inputs` fail BCS canonicalization
//! fail-closed. The protocol bans floating-point economic state (§10.3).
//!
//! The `Intent` struct deliberately excludes the `signature` field: per the
//! ADR-004 structural envelope rule (§2) a signature covers only the body, so
//! signatures live in [`crate::signed::Signed<Intent>`] instead of inside the
//! body type. Nonces use the protocol's `b64u:` base64url-unpadded string form
//! (§11.1, §17).

use core::fmt;
use std::collections::BTreeMap;
use std::str::FromStr;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, Utc};
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use statechronicle_core::limits::MAX_ID_LENGTH;
use statechronicle_core::signature::Signature;

use crate::authority::AuthorityProof;
use crate::error::DomainError;
use crate::ids::IntentId;
use crate::resource::ResourceId;
use crate::state_type::StateType;
use crate::subject::SubjectId;
use crate::tenant::TenantId;

/// Schema identifier for v0 intents (protocol §11.1).
pub const INTENT_SCHEMA: &str = "statechronicle.intent.v0";

/// Maximum decoded byte length of a nonce (protocol §11.1).
pub const MAX_NONCE_BYTES: usize = 64;

/// The `b64u:` prefix used by the protocol's nonce string form (§17).
const NONCE_PREFIX: &str = "b64u:";

/// A registry-open operation name (profile-defined), e.g. `asset.transfer`.
///
/// Operations are registered strings, not a closed enum, so profiles can
/// define new operations without a protocol schema bump (protocol §11.1).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Operation(pub String);

impl Operation {
    /// Constructs a validated operation name.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::InvalidOperation`] when `value` is empty or
    /// exceeds [`MAX_ID_LENGTH`] characters.
    pub fn new(value: String) -> Result<Self, DomainError> {
        validate_operation(&value)?;
        Ok(Self(value))
    }

    /// Returns the operation name as a string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Operation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for Operation {
    type Err = DomainError;

    /// Parses an operation name, validating it.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::InvalidOperation`] when the string is empty or
    /// exceeds [`MAX_ID_LENGTH`] characters.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(String::from(s))
    }
}

impl TryFrom<String> for Operation {
    type Error = DomainError;

    /// Converts an owned string into a validated operation name.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::InvalidOperation`] when the string is empty or
    /// exceeds [`MAX_ID_LENGTH`] characters.
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl TryFrom<&str> for Operation {
    type Error = DomainError;

    /// Converts a borrowed string into a validated operation name.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::InvalidOperation`] when the string is empty or
    /// exceeds [`MAX_ID_LENGTH`] characters.
    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::new(String::from(value))
    }
}

impl Serialize for Operation {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for Operation {
    /// Deserializes an operation name, validating it.
    ///
    /// # Errors
    ///
    /// Returns a serde error when the string is empty or exceeds
    /// [`MAX_ID_LENGTH`] characters.
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(D::Error::custom)
    }
}

/// Validates an operation name: non-empty and within the id length bound.
///
/// # Errors
///
/// Returns [`DomainError::InvalidOperation`] when `value` is empty or exceeds
/// [`MAX_ID_LENGTH`] characters.
fn validate_operation(value: &str) -> Result<(), DomainError> {
    if value.is_empty() {
        return Err(DomainError::InvalidOperation(String::from(
            "operation must not be empty",
        )));
    }
    if value.len() > MAX_ID_LENGTH {
        return Err(DomainError::InvalidOperation(format!(
            "operation must be at most {MAX_ID_LENGTH} chars, got {}",
            value.len()
        )));
    }
    Ok(())
}

/// A replay-protection nonce in the protocol's `b64u:` string form.
///
/// Stores the decoded bytes; the `b64u:<base64url-unpadded>` string form is
/// used for `Display`, serde, and the wire format (§11.1, §17).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Nonce {
    bytes: Vec<u8>,
}

impl Nonce {
    /// Parses a `b64u:` base64url-unpadded nonce string.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::InvalidNonce`] when `value` does not start with
    /// `b64u:`, is not valid base64url, or decodes to more than
    /// [`MAX_NONCE_BYTES`] bytes.
    pub fn from_b64u_str(value: &str) -> Result<Self, DomainError> {
        let encoded = value.strip_prefix(NONCE_PREFIX).ok_or_else(|| {
            DomainError::InvalidNonce(format!("nonce must start with `{NONCE_PREFIX}`"))
        })?;
        let decoded = URL_SAFE_NO_PAD.decode(encoded).map_err(|source| {
            DomainError::InvalidNonce(format!("nonce is not valid base64url: {source}"))
        })?;
        Self::from_bytes(decoded)
    }

    /// Constructs a nonce from raw bytes.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::InvalidNonce`] when `bytes` exceeds
    /// [`MAX_NONCE_BYTES`] in length.
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, DomainError> {
        if bytes.len() > MAX_NONCE_BYTES {
            return Err(DomainError::InvalidNonce(format!(
                "nonce must be at most {MAX_NONCE_BYTES} bytes, got {}",
                bytes.len()
            )));
        }
        Ok(Self { bytes })
    }

    /// Returns the raw nonce bytes.
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Encodes the nonce in the `b64u:` base64url-unpadded string form.
    pub fn to_b64u_string(&self) -> String {
        format!("{NONCE_PREFIX}{}", URL_SAFE_NO_PAD.encode(&self.bytes))
    }
}

impl fmt::Display for Nonce {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_b64u_string())
    }
}

impl Serialize for Nonce {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_b64u_string())
    }
}

impl<'de> Deserialize<'de> for Nonce {
    /// Deserializes from the `b64u:` string form, validating it.
    ///
    /// # Errors
    ///
    /// Returns a serde error when the string is not a valid nonce.
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Self::from_b64u_str(&value).map_err(D::Error::custom)
    }
}

/// Signature algorithm identifiers (ADR-004 §5).
///
/// Ed25519 is the v0 baseline; additional algorithms are additive enum
/// variants behind a schema bump.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SignatureAlg {
    /// Ed25519 over BCS canonical bytes (ADR-004 §5).
    #[serde(rename = "ed25519")]
    Ed25519,
}

/// A signing-key identifier (protocol §11.1 signature block).
///
/// The `did:key:` convention from the protocol examples is documented but not
/// enforced at the domain layer. Only non-emptiness and length are validated.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct KeyId(pub String);

impl KeyId {
    /// Constructs a validated signing-key identifier.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::InvalidKeyId`] when `value` is empty or exceeds
    /// [`MAX_ID_LENGTH`] characters.
    pub fn new(value: String) -> Result<Self, DomainError> {
        validate_key_id(&value)?;
        Ok(Self(value))
    }

    /// Returns the key id as a string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for KeyId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for KeyId {
    type Err = DomainError;

    /// Parses a key id, validating it.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::InvalidKeyId`] when the string is empty or
    /// exceeds [`MAX_ID_LENGTH`] characters.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(String::from(s))
    }
}

impl TryFrom<String> for KeyId {
    type Error = DomainError;

    /// Converts an owned string into a validated key id.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::InvalidKeyId`] when the string is empty or
    /// exceeds [`MAX_ID_LENGTH`] characters.
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl TryFrom<&str> for KeyId {
    type Error = DomainError;

    /// Converts a borrowed string into a validated key id.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::InvalidKeyId`] when the string is empty or
    /// exceeds [`MAX_ID_LENGTH`] characters.
    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::new(String::from(value))
    }
}

impl Serialize for KeyId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for KeyId {
    /// Deserializes a key id, validating it.
    ///
    /// # Errors
    ///
    /// Returns a serde error when the string is empty or exceeds
    /// [`MAX_ID_LENGTH`] characters.
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(D::Error::custom)
    }
}

/// Validates a key id: non-empty and within the id length bound.
///
/// # Errors
///
/// Returns [`DomainError::InvalidKeyId`] when `value` is empty or exceeds
/// [`MAX_ID_LENGTH`] characters.
fn validate_key_id(value: &str) -> Result<(), DomainError> {
    if value.is_empty() {
        return Err(DomainError::InvalidKeyId(String::from(
            "key id must not be empty",
        )));
    }
    if value.len() > MAX_ID_LENGTH {
        return Err(DomainError::InvalidKeyId(format!(
            "key id must be at most {MAX_ID_LENGTH} chars, got {}",
            value.len()
        )));
    }
    Ok(())
}

/// A detached signature block (ADR-004 structural envelope).
///
/// The signature covers only the BCS canonical bytes of the signed body, never
/// this block itself (ADR-004 §2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignatureBlock {
    /// Signature algorithm used to produce `sig`.
    pub alg: SignatureAlg,
    /// Identifier of the signing key.
    pub key_id: KeyId,
    /// The Ed25519 signature bytes in `b64u:` form.
    pub sig: Signature,
}

/// A requested state transition (protocol §11.1).
///
/// The `signature` field is deliberately excluded from this body type: per the
/// ADR-004 structural envelope rule (§2), signatures live in
/// [`crate::signed::Signed<Intent>`] so a signature never covers a signature
/// field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Intent {
    /// Schema identifier, always [`INTENT_SCHEMA`] for v0.
    pub schema: String,
    /// The tenant scope of the transition.
    pub tenant_id: TenantId,
    /// Unique intent id, used for idempotency (protocol §11.2).
    pub intent_id: IntentId,
    /// The requested operation (profile-defined).
    pub operation: Operation,
    /// The actor requesting the transition.
    pub actor: SubjectId,
    /// The resource being mutated.
    pub resource_id: ResourceId,
    /// State type when required by the active profile.
    pub state_type: Option<StateType>,
    /// The expected version of the resource state (optimistic concurrency).
    pub expected_version: u64,
    /// Profile-defined operation inputs, key-sorted for canonicalization.
    pub inputs: BTreeMap<String, serde_json::Value>,
    /// Optional authority proof binding a TrustGrant evaluation.
    pub authority: Option<AuthorityProof>,
    /// When the intent was created (UTC).
    pub created_at: DateTime<Utc>,
    /// When the intent expires, if any.
    pub expires_at: Option<DateTime<Utc>>,
    /// Replay-protection nonce in `b64u:` form.
    pub nonce: Nonce,
}

impl Intent {
    /// Constructs an intent with the v0 schema identifier set.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        tenant_id: TenantId,
        intent_id: IntentId,
        operation: Operation,
        actor: SubjectId,
        resource_id: ResourceId,
        state_type: Option<StateType>,
        expected_version: u64,
        inputs: BTreeMap<String, serde_json::Value>,
        authority: Option<AuthorityProof>,
        created_at: DateTime<Utc>,
        expires_at: Option<DateTime<Utc>>,
        nonce: Nonce,
    ) -> Self {
        Self {
            schema: String::from(INTENT_SCHEMA),
            tenant_id,
            intent_id,
            operation,
            actor,
            resource_id,
            state_type,
            expected_version,
            inputs,
            authority,
            created_at,
            expires_at,
            nonce,
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;
    use statechronicle_core::signature::Signature;

    fn sample_nonce() -> Nonce {
        Nonce::from_bytes(vec![1, 2, 3, 4]).unwrap()
    }

    fn sample_inputs() -> BTreeMap<String, serde_json::Value> {
        BTreeMap::from([
            (
                String::from("from_owner"),
                serde_json::json!("account:stexs:player_123"),
            ),
            (
                String::from("to_owner"),
                serde_json::json!("account:stexs:player_456"),
            ),
        ])
    }

    fn sample_intent() -> Intent {
        Intent::new(
            TenantId(String::from("stexs.game.alpha")),
            IntentId::new(String::from("int_01JZ8WJ1V6MJ6Y3Z6Z9CA8B2K2")).unwrap(),
            Operation::new(String::from("asset.transfer")).unwrap(),
            SubjectId(String::from("account:stexs:player_123")),
            ResourceId(String::from("asset:sword_001")),
            Some(StateType::UniqueAsset),
            41,
            sample_inputs(),
            None,
            chrono::DateTime::parse_from_rfc3339("2026-07-14T00:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
            None,
            sample_nonce(),
        )
    }

    #[test]
    fn constructor_sets_schema() {
        let intent = sample_intent();
        assert_eq!(intent.schema, INTENT_SCHEMA);
        assert_eq!(intent.expected_version, 41);
    }

    #[test]
    fn serde_json_roundtrips() {
        let intent = sample_intent();
        let json = serde_json::to_string(&intent).unwrap();
        let decoded: Intent = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, intent);
    }

    #[test]
    fn bcs_roundtrips_with_empty_inputs() {
        // `inputs` holds `serde_json::Value`, which is BCS-encodable but not
        // BCS-decodable (BCS is not self-describing, ADR-004). With an empty
        // inputs map the rest of the intent round-trips fully through BCS.
        let mut intent = sample_intent();
        intent.inputs = BTreeMap::new();
        let bytes = bcs::to_bytes(&intent).unwrap();
        let decoded: Intent = bcs::from_bytes(&bytes).unwrap();
        assert_eq!(decoded, intent);
    }

    #[test]
    fn bcs_canonicalization_is_deterministic_with_inputs() {
        // Profile-defined string inputs canonicalize deterministically, which
        // is what hashing and signing depend on.
        let intent = sample_intent();
        let first = bcs::to_bytes(&intent).unwrap();
        let second = bcs::to_bytes(&intent).unwrap();
        assert_eq!(first, second);
        assert!(!first.is_empty());
    }

    #[test]
    fn bcs_rejects_float_inputs_fail_closed() {
        // Floats are structurally banned from canonical state (§10.3): BCS has
        // no float encoding, so canonicalization fails closed.
        let mut intent = sample_intent();
        intent.inputs = BTreeMap::from([(String::from("amount"), serde_json::json!(1.5))]);
        assert!(bcs::to_bytes(&intent).is_err());
    }

    #[test]
    fn nonce_roundtrips_through_b64u_string() {
        let nonce = sample_nonce();
        let encoded = nonce.to_b64u_string();
        assert!(encoded.starts_with("b64u:"));
        assert_eq!(Nonce::from_b64u_str(&encoded).unwrap(), nonce);
        assert_eq!(nonce.as_bytes(), &[1, 2, 3, 4]);
    }

    #[test]
    fn nonce_rejects_bad_prefix() {
        assert!(matches!(
            Nonce::from_b64u_str("QUJD"),
            Err(DomainError::InvalidNonce(_))
        ));
    }

    #[test]
    fn nonce_rejects_bad_base64() {
        assert!(matches!(
            Nonce::from_b64u_str("b64u:%%%"),
            Err(DomainError::InvalidNonce(_))
        ));
    }

    #[test]
    fn nonce_rejects_too_long() {
        assert!(Nonce::from_bytes(vec![0u8; MAX_NONCE_BYTES.saturating_add(1)]).is_err());
    }

    #[test]
    fn operation_validates_empty_and_length() {
        assert!(Operation::new(String::new()).is_err());
        assert!(matches!(
            Operation::from_str(""),
            Err(DomainError::InvalidOperation(_))
        ));
        assert!(Operation::new("x".repeat(MAX_ID_LENGTH.saturating_add(1))).is_err());
        assert!(Operation::new(String::from("asset.transfer")).is_ok());
    }

    #[test]
    fn key_id_validates_empty_and_length() {
        assert!(KeyId::new(String::new()).is_err());
        assert!(KeyId::new("x".repeat(MAX_ID_LENGTH.saturating_add(1))).is_err());
        assert!(KeyId::new(String::from("did:key:z6Mk...#key-1")).is_ok());
    }

    #[test]
    fn signature_block_serde_roundtrips() {
        let block = SignatureBlock {
            alg: SignatureAlg::Ed25519,
            key_id: KeyId::new(String::from("did:key:z6Mk...#key-1")).unwrap(),
            sig: Signature::from_bytes([0u8; 64]),
        };
        let json = serde_json::to_string(&block).unwrap();
        assert!(json.contains("\"ed25519\""));
        let decoded: SignatureBlock = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, block);
    }
}
