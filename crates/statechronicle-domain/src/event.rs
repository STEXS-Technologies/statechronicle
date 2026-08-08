//! Events: validated, append-only transitions (protocol §12).
//!
//! Events are emitted only after the execution pipeline passes every check.
//! They are the unit of replay and carry TrustGrant evaluation bindings. Each
//! event references exactly one accepted intent and one resource, carries
//! before/after state commitments, and is assigned to exactly one commit
//! (protocol §12.2). Events are not individually signed; they are covered by
//! the enclosing signed commit.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use statechronicle_core::digest::ContentDigest;

use crate::authority::AuthorityProof;
use crate::ids::{EventId, IntentId};
use crate::intent::Operation;
use crate::resource::ResourceId;
use crate::subject::SubjectId;
use crate::tenant::TenantId;

/// Schema identifier for v0 events (protocol §12.1).
pub const EVENT_SCHEMA: &str = "statechronicle.event.v0";

/// A before/after state commitment bound into an event.
///
/// `state` is the profile projection payload (owner/status, balance/unit,
/// quantity, ...) and `state_hash` is its canonical content digest; together
/// they let a verifier check the transition without the full history
/// (protocol §12.1, ADR-004 §4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateCommitment {
    /// Resource version after this commitment.
    pub version: u64,
    /// Canonical digest of the projected state.
    pub state_hash: ContentDigest,
    /// The profile-defined projected state payload.
    pub state: serde_json::Value,
}

/// A validated, append-only state transition (protocol §12.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event {
    /// Schema identifier, always [`EVENT_SCHEMA`] for v0.
    pub schema: String,
    /// The tenant scope in which the event was executed.
    pub tenant_id: TenantId,
    /// Unique event id.
    pub event_id: EventId,
    /// The accepted intent that produced this event.
    pub intent_id: IntentId,
    /// The operation that was executed.
    pub operation: Operation,
    /// The resource that was mutated.
    pub resource_id: ResourceId,
    /// The actor who authorized the transition.
    pub actor: SubjectId,
    /// State before the transition.
    pub before: StateCommitment,
    /// State after the transition.
    pub after: StateCommitment,
    /// Optional authority proof binding a TrustGrant evaluation.
    pub authority: Option<AuthorityProof>,
    /// The executor that validated and emitted this event.
    pub executor: SubjectId,
    /// When the event was created (UTC).
    pub created_at: DateTime<Utc>,
}

impl Event {
    /// Constructs an event with the v0 schema identifier set.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        tenant_id: TenantId,
        event_id: EventId,
        intent_id: IntentId,
        operation: Operation,
        resource_id: ResourceId,
        actor: SubjectId,
        before: StateCommitment,
        after: StateCommitment,
        authority: Option<AuthorityProof>,
        executor: SubjectId,
        created_at: DateTime<Utc>,
    ) -> Self {
        Self {
            schema: String::from(EVENT_SCHEMA),
            tenant_id,
            event_id,
            intent_id,
            operation,
            resource_id,
            actor,
            before,
            after,
            authority,
            executor,
            created_at,
        }
    }
}
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::authority::{AuthorityProof, EvaluationResult, TRUSTGRANT_EVALUATION_KIND};
    use statechronicle_core::canonicalize::canonicalize_and_digest;
    use statechronicle_core::digest::hash_bytes;

    fn sample_commitment(version: u64, owner: &str) -> StateCommitment {
        let state = serde_json::json!({ "owner": owner, "status": "active" });
        let digest = canonicalize_and_digest(&state).unwrap();
        StateCommitment {
            version,
            state_hash: digest,
            state,
        }
    }

    fn sample_event() -> Event {
        Event::new(
            TenantId(String::from("acme.game.alpha")),
            EventId::new(String::from("evt_01JZ8X2XRE5ZYW5V9R7VDQBSH4")).unwrap(),
            IntentId::new(String::from("int_01JZ8WJ1V6MJ6Y3Z6Z9CA8B2K2")).unwrap(),
            Operation::new(String::from("asset.transfer")).unwrap(),
            ResourceId(String::from("asset:sword_001")),
            SubjectId(String::from("account:example:player_123")),
            sample_commitment(41, "account:example:player_123"),
            sample_commitment(42, "account:example:player_456"),
            None,
            SubjectId(String::from("service:statechronicle.example.net")),
            DateTime::parse_from_rfc3339("2026-07-14T00:00:01Z")
                .unwrap()
                .with_timezone(&Utc),
        )
    }

    #[test]
    fn constructor_sets_schema() {
        let event = sample_event();
        assert_eq!(event.schema, EVENT_SCHEMA);
        assert_eq!(event.before.version, 41);
        assert_eq!(event.after.version, 42);
    }

    #[test]
    fn serde_json_roundtrips() {
        let event = sample_event();
        let json = serde_json::to_string(&event).unwrap();
        let decoded: Event = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, event);
    }

    #[test]
    fn bcs_canonicalization_is_deterministic() {
        // The `state` payload is `serde_json::Value`, which is BCS-encodable
        // but not BCS-decodable (BCS is not self-describing, ADR-004), so the
        // BCS check is encode-side determinism, the property signing relies on.
        let event = sample_event();
        let first = bcs::to_bytes(&event).unwrap();
        let second = bcs::to_bytes(&event).unwrap();
        assert_eq!(first, second);
        assert!(!first.is_empty());
    }

    #[test]
    fn authority_present_roundtrips() {
        let mut event = sample_event();
        event.authority = Some(AuthorityProof {
            kind: String::from(TRUSTGRANT_EVALUATION_KIND),
            evaluation_digest: hash_bytes(b"trustgrant"),
            result: EvaluationResult::Allow,
            evaluated_at: DateTime::parse_from_rfc3339("2026-07-14T00:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
        });
        let json = serde_json::to_string(&event).unwrap();
        let decoded: Event = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, event);
    }
}
