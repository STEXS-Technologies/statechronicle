//! The validated `ValidatedIntent`.
//!
//! Produced by the validation stage from a `RawIntent`; carries the parsed,
//! canonical form and intent id.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use statechronicle_core::canonicalize::{canonicalize, canonicalize_and_digest};
use statechronicle_core::digest::ContentDigest;
use statechronicle_core::error::StateChronicleError;
use statechronicle_core::limits::{MAX_ID_LENGTH, MAX_INTENT_BYTES, MAX_JSON_DEPTH};

use statechronicle_domain::ids::IntentId;
use statechronicle_domain::intent::{Intent, Operation, SignatureBlock};
use statechronicle_domain::resource::ResourceId;
use statechronicle_domain::subject::SubjectId;
use statechronicle_domain::tenant::TenantId;

/// The §11.2 idempotency tuple.
///
/// The protocol requires the tuple `(tenant_id, intent_id, actor,
/// resource_id, operation)` to be idempotent: replaying the same accepted
/// intent must return the same committed result, and replaying a conflicting
/// intent with the same `intent_id` must fail. This key is the canonical
/// representation of that tuple for store lookups and deduplication.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct IdempotencyKey {
    /// The tenant scope of the transition.
    pub tenant_id: TenantId,
    /// The unique intent id.
    pub intent_id: IntentId,
    /// The actor requesting the transition.
    pub actor: SubjectId,
    /// The resource being mutated.
    pub resource_id: ResourceId,
    /// The requested operation.
    pub operation: Operation,
}

impl IdempotencyKey {
    /// Constructs an idempotency key from the validated intent fields.
    pub const fn new(
        tenant_id: TenantId,
        intent_id: IntentId,
        actor: SubjectId,
        resource_id: ResourceId,
        operation: Operation,
    ) -> Self {
        Self {
            tenant_id,
            intent_id,
            actor,
            resource_id,
            operation,
        }
    }
}

/// A validated intent: the parsed canonical body plus its idempotency key.
///
/// Produced by [`crate::validate::validate`] after the schema check, newtype
/// construction, expiry check, and optional signature parsing. The `intent`
/// is the canonical domain form; `signature`, when present, is the detached
/// signature block over the intent's canonical bytes (ADR-004 §2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidatedIntent {
    /// The parsed, canonical intent body.
    pub intent: Intent,
    /// The §11.2 idempotency tuple derived from the intent.
    pub idempotency_key: IdempotencyKey,
    /// The optional detached signature block parsed from the raw payload.
    pub signature: Option<SignatureBlock>,
}

impl ValidatedIntent {
    /// Validates an already-typed intent before wrapping it for execution.
    /// This is the safe constructor for transport adapters that deserialize
    /// domain objects directly instead of going through [`crate::parse`].
    ///
    /// # Errors
    ///
    /// Returns [`crate::error::IntentError`] when schema, identity, expiry, or
    /// canonical serialized-size invariants are invalid.
    pub fn try_from_intent(
        intent: Intent,
        signature: Option<SignatureBlock>,
    ) -> Result<Self, crate::error::IntentError> {
        if intent.schema != statechronicle_domain::intent::INTENT_SCHEMA {
            return Err(crate::error::IntentError::InvalidSchema {
                found: intent.schema,
                expected: String::from(statechronicle_domain::intent::INTENT_SCHEMA),
            });
        }
        for (name, value) in [
            ("tenant", intent.tenant_id.0.as_str()),
            ("actor", intent.actor.0.as_str()),
            ("resource", intent.resource_id.0.as_str()),
        ] {
            if value.is_empty() {
                return Err(crate::error::IntentError::InvalidField(format!(
                    "{name} identifier must not be empty"
                )));
            }
            if value.chars().count() > MAX_ID_LENGTH {
                return Err(crate::error::IntentError::InvalidField(format!(
                    "{name} identifier exceeds {MAX_ID_LENGTH} characters"
                )));
            }
            if value.chars().any(char::is_control) {
                return Err(crate::error::IntentError::InvalidField(format!(
                    "{name} identifier must not contain control characters"
                )));
            }
        }
        if intent
            .expires_at
            .is_some_and(|expiry| expiry <= intent.created_at)
        {
            return Err(crate::error::IntentError::InvalidExpiry(String::from(
                "expires_at must be after created_at",
            )));
        }
        let input_depth = intent.inputs.values().map(json_depth).max().unwrap_or(0);
        if input_depth > MAX_JSON_DEPTH {
            return Err(crate::error::IntentError::InvalidField(format!(
                "intent input nesting depth exceeds limit {MAX_JSON_DEPTH}"
            )));
        }
        let size = canonicalize(&intent)?.len();
        if size > MAX_INTENT_BYTES {
            return Err(crate::error::IntentError::SizeLimitExceeded {
                name: String::from("intent"),
                limit: MAX_INTENT_BYTES,
                actual: size,
            });
        }
        Ok(Self::from_intent(intent, signature))
    }

    /// Constructs a validated intent directly from an already-typed intent
    /// body, skipping raw-payload parsing.
    ///
    /// Use this entry point when the intent is already in typed domain form
    /// (for example, built in-process by the caller, or deserialized by the
    /// caller's own transport layer), so no [`crate::parse`] step is needed.
    /// The §11.2 idempotency tuple is derived from the intent's fields.
    ///
    /// This is a trusted/internal constructor and does not repeat structural
    /// checks; transport adapters should use [`Self::try_from_intent`] instead.
    /// It complements [`crate::validate::validate`], which is the entry point
    /// for raw wire payloads.
    pub fn from_intent(intent: Intent, signature: Option<SignatureBlock>) -> Self {
        let idempotency_key = IdempotencyKey::new(
            intent.tenant_id.clone(),
            intent.intent_id.clone(),
            intent.actor.clone(),
            intent.resource_id.clone(),
            intent.operation.clone(),
        );
        Self {
            intent,
            idempotency_key,
            signature,
        }
    }

    /// Returns whether the intent has expired at `now`.
    ///
    /// An intent without an expiry never expires. An intent is expired when
    /// `expires_at` is present and not strictly after `now` (protocol §11.1).
    pub fn is_expired(&self, now: DateTime<Utc>) -> bool {
        self.intent.expires_at.is_some_and(|expiry| expiry <= now)
    }

    /// Computes the canonical digest that must be bound to a durable
    /// idempotency reservation.  The digest covers the complete typed intent,
    /// including inputs, nonce, expiry, and authority—not merely `intent_id`.
    ///
    /// # Errors
    ///
    /// Returns a canonicalization error when the intent cannot be serialized
    /// to its protocol BCS representation.
    pub fn payload_digest(&self) -> Result<ContentDigest, StateChronicleError> {
        canonicalize_and_digest(&self.intent)
    }
}

fn json_depth(value: &serde_json::Value) -> usize {
    match value {
        serde_json::Value::Array(values) => {
            1usize.saturating_add(values.iter().map(json_depth).max().unwrap_or(0))
        }
        serde_json::Value::Object(values) => {
            1usize.saturating_add(values.values().map(json_depth).max().unwrap_or(0))
        }
        serde_json::Value::Null
        | serde_json::Value::Bool(_)
        | serde_json::Value::Number(_)
        | serde_json::Value::String(_) => 0,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;
    use statechronicle_core::signature::Signature;
    use statechronicle_domain::intent::Nonce;
    use statechronicle_domain::state_type::StateType;

    fn sample_key() -> IdempotencyKey {
        IdempotencyKey::new(
            TenantId(String::from("acme.game.alpha")),
            IntentId::new(String::from("int_01JZ8WJ1V6MJ6Y3Z6Z9CA8B2K2")).unwrap(),
            SubjectId(String::from("account:example:player_123")),
            ResourceId(String::from("asset:sword_001")),
            Operation::new(String::from("asset.transfer")).unwrap(),
        )
    }

    #[test]
    fn idempotency_key_roundtrips_through_json() {
        let key = sample_key();
        let json = serde_json::to_string(&key).unwrap();
        let decoded: IdempotencyKey = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, key);
    }

    #[test]
    fn idempotency_key_hashes() {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut first_hasher = DefaultHasher::new();
        sample_key().hash(&mut first_hasher);
        let first = first_hasher.finish();

        let mut second_hasher = DefaultHasher::new();
        sample_key().hash(&mut second_hasher);
        let second = second_hasher.finish();
        assert_eq!(first, second);
    }

    #[test]
    fn idempotency_key_differs_on_intent_id() {
        let key = sample_key();
        let other = IdempotencyKey::new(
            key.tenant_id.clone(),
            IntentId::new(String::from("int_other")).unwrap(),
            key.actor.clone(),
            key.resource_id.clone(),
            key.operation.clone(),
        );
        assert_ne!(key, other);
    }

    #[test]
    fn from_intent_derives_idempotency_key_without_parsing() {
        // A consumer with already-typed data builds the ValidatedIntent
        // directly, skipping the raw-payload parse step entirely.
        let intent = Intent::new(
            TenantId(String::from("acme.game.alpha")),
            IntentId::new(String::from("int_01JZ8WJ1V6MJ6Y3Z6Z9CA8B2K2")).unwrap(),
            Operation::new(String::from("asset.transfer")).unwrap(),
            SubjectId(String::from("account:example:player_123")),
            ResourceId(String::from("asset:sword_001")),
            Some(StateType::UniqueAsset),
            41,
            std::collections::BTreeMap::new(),
            None,
            DateTime::parse_from_rfc3339("2026-07-14T00:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
            None,
            Nonce::from_bytes(vec![1, 2, 3]).unwrap(),
        );
        let validated = ValidatedIntent::from_intent(intent.clone(), None);
        assert_eq!(validated.intent, intent);
        assert_eq!(validated.signature, None);
        assert_eq!(validated.idempotency_key, sample_key());
    }

    #[test]
    fn try_from_intent_rejects_invalid_typed_fields() {
        let mut intent = sample_intent();
        intent.actor = SubjectId(String::new());
        assert!(ValidatedIntent::try_from_intent(intent, None).is_err());
    }

    #[test]
    fn try_from_intent_rejects_unbounded_or_control_identifiers() {
        let mut oversized = sample_intent();
        oversized.tenant_id = TenantId("é".repeat(MAX_ID_LENGTH + 1));
        assert!(ValidatedIntent::try_from_intent(oversized, None).is_err());

        let mut control = sample_intent();
        control.resource_id = ResourceId(String::from("asset:bad\nvalue"));
        assert!(ValidatedIntent::try_from_intent(control, None).is_err());
    }

    #[test]
    fn try_from_intent_rejects_deep_typed_inputs() {
        let mut intent = sample_intent();
        let mut value = serde_json::Value::Null;
        for _ in 0..=MAX_JSON_DEPTH {
            value = serde_json::Value::Array(vec![value]);
        }
        intent.inputs.insert(String::from("nested"), value);
        assert!(ValidatedIntent::try_from_intent(intent, None).is_err());
    }

    #[test]
    fn signature_block_field_is_serializable() {
        let block = SignatureBlock {
            alg: statechronicle_domain::intent::SignatureAlg::Ed25519,
            key_id: statechronicle_domain::intent::KeyId::new(String::from(
                "did:key:z6Mk...#key-1",
            ))
            .unwrap(),
            sig: Signature::from_bytes([0u8; 64]),
        };
        let json = serde_json::to_string(&block).unwrap();
        let decoded: SignatureBlock = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, block);
    }

    #[test]
    fn payload_digest_binds_full_intent_not_only_intent_id() {
        let base = ValidatedIntent::from_intent(sample_intent(), None);
        let mut changed = base.intent.clone();
        changed
            .inputs
            .insert(String::from("amount"), serde_json::json!("1"));
        let changed = ValidatedIntent::from_intent(changed, None);
        assert_ne!(
            base.payload_digest().unwrap(),
            changed.payload_digest().unwrap()
        );
    }

    fn sample_intent() -> Intent {
        Intent::new(
            TenantId(String::from("acme.game.alpha")),
            IntentId::new(String::from("int_01JZ8WJ1V6MJ6Y3Z6Z9CA8B2K2")).unwrap(),
            Operation::new(String::from("asset.transfer")).unwrap(),
            SubjectId(String::from("account:example:player_123")),
            ResourceId(String::from("asset:sword_001")),
            Some(StateType::UniqueAsset),
            41,
            std::collections::BTreeMap::new(),
            None,
            DateTime::parse_from_rfc3339("2026-07-14T00:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
            None,
            Nonce::from_bytes(vec![1, 2, 3]).unwrap(),
        )
    }
}
