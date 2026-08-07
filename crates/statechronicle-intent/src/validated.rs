//! The validated `ValidatedIntent`.
//!
//! Produced by the validation stage from a `RawIntent`; carries the parsed,
//! canonical form and intent id.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

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
    /// Returns whether the intent has expired at `now`.
    ///
    /// An intent without an expiry never expires. An intent is expired when
    /// `expires_at` is present and not strictly after `now` (protocol §11.1).
    pub fn is_expired(&self, now: DateTime<Utc>) -> bool {
        self.intent.expires_at.is_some_and(|expiry| expiry <= now)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;
    use statechronicle_core::signature::Signature;

    fn sample_key() -> IdempotencyKey {
        IdempotencyKey::new(
            TenantId(String::from("stexs.game.alpha")),
            IntentId::new(String::from("int_01JZ8WJ1V6MJ6Y3Z6Z9CA8B2K2")).unwrap(),
            SubjectId(String::from("account:stexs:player_123")),
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
}
