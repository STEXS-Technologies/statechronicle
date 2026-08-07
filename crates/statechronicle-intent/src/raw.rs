//! The unvalidated `RawIntent` document.
//!
//! `RawIntent` is the client-submitted payload before schema validation and
//! canonical parsing (trustgrant stage separation).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The client-submitted intent payload before schema validation (protocol
/// §11.1).
///
/// Field values are kept in their raw wire form — strings, numbers, and
/// unparsed JSON values — so the parse stage only needs to prove the payload
/// is JSON-shaped. Typed construction happens in [`crate::validate`], where
/// every id, nonce, timestamp, and proof is checked against its protocol
/// constraints and failures are reported as field-level errors.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawIntent {
    /// Schema identifier, checked against [`INTENT_SCHEMA`] in validation.
    pub schema: String,
    /// Tenant scope of the transition (§11.1).
    pub tenant_id: String,
    /// Unique intent id, used for idempotency (§11.2).
    pub intent_id: String,
    /// The requested operation (profile-defined, §11.1).
    pub operation: String,
    /// The actor requesting the transition.
    pub actor: String,
    /// The resource being mutated.
    pub resource_id: String,
    /// State type when required by the active profile; parsed in validation.
    pub state_type: Option<serde_json::Value>,
    /// The expected version of the resource state (optimistic concurrency).
    pub expected_version: u64,
    /// Profile-defined operation inputs; absent payloads default to empty.
    #[serde(default)]
    pub inputs: BTreeMap<String, serde_json::Value>,
    /// Optional authority proof; parsed in validation.
    pub authority: Option<serde_json::Value>,
    /// When the intent was created (RFC 3339, UTC).
    pub created_at: String,
    /// When the intent expires, if any (RFC 3339, UTC).
    pub expires_at: Option<String>,
    /// Replay-protection nonce in the protocol's `b64u:` form (§17).
    pub nonce: String,
    /// Optional detached signature block over the canonical intent body
    /// (ADR-004 §2).
    pub signature: Option<RawSignature>,
}

/// The raw signature block as submitted by the client (ADR-004 §2).
///
/// Kept as raw strings so the parse stage stays lenient; the block is parsed
/// into a [`statechronicle_domain::intent::SignatureBlock`] during validation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawSignature {
    /// Signature algorithm identifier, e.g. `ed25519`.
    pub alg: String,
    /// Identifier of the signing key.
    pub key_id: String,
    /// The signature bytes in the protocol's `b64u:` form (§17).
    pub sig: String,
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;
    use statechronicle_domain::intent::INTENT_SCHEMA;

    fn sample_raw() -> RawIntent {
        RawIntent {
            schema: String::from(INTENT_SCHEMA),
            tenant_id: String::from("stexs.game.alpha"),
            intent_id: String::from("int_01JZ8WJ1V6MJ6Y3Z6Z9CA8B2K2"),
            operation: String::from("asset.transfer"),
            actor: String::from("account:stexs:player_123"),
            resource_id: String::from("asset:sword_001"),
            state_type: Some(serde_json::json!("unique_asset")),
            expected_version: 41,
            inputs: BTreeMap::from([(
                String::from("to_owner"),
                serde_json::json!("account:stexs:player_456"),
            )]),
            authority: None,
            created_at: String::from("2026-07-14T00:00:00Z"),
            expires_at: Some(String::from("2026-07-14T00:05:00Z")),
            nonce: String::from("b64u:AAME"),
            signature: None,
        }
    }

    #[test]
    fn raw_intent_roundtrips_through_json() {
        let raw = sample_raw();
        let json = serde_json::to_string(&raw).unwrap();
        let decoded: RawIntent = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, raw);
    }

    #[test]
    fn absent_optional_fields_default_to_none_and_empty() {
        let value = serde_json::json!({
            "schema": INTENT_SCHEMA,
            "tenant_id": "t",
            "intent_id": "int_x",
            "operation": "op",
            "actor": "a",
            "resource_id": "r",
            "expected_version": 0,
            "created_at": "2026-07-14T00:00:00Z",
            "nonce": "b64u:AAME",
        });
        let raw: RawIntent = serde_json::from_value(value).unwrap();
        assert!(raw.inputs.is_empty());
        assert!(raw.state_type.is_none());
        assert!(raw.expires_at.is_none());
        assert!(raw.authority.is_none());
        assert!(raw.signature.is_none());
    }

    #[test]
    fn raw_signature_roundtrips_through_json() {
        let signature = RawSignature {
            alg: String::from("ed25519"),
            key_id: String::from("did:key:z6Mk...#key-1"),
            sig: format!("b64u:{}", "A".repeat(86)),
        };
        let json = serde_json::to_string(&signature).unwrap();
        let decoded: RawSignature = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, signature);
    }
}
