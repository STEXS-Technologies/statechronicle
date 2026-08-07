//! Authority proofs and TrustGrant evaluation outcomes.
//!
//! Authority enters the ledger as opaque, content-addressed evidence:
//! `{ kind, evaluation_digest, result }` (protocol §12.1). The ledger never
//! parses the internal structure of a trustgrant evaluation — it binds the
//! digest and result into signed events, and verifiers resolve the digest
//! through their own trustgrant integration (protocol §16.3, §29).

use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use statechronicle_core::digest::{ContentDigest, hash_bytes};

/// Registered authority kind for TrustGrant evaluations.
pub const TRUSTGRANT_EVALUATION_KIND: &str = "trustgrant.evaluation";

/// Domain tag of the multi-authority aggregation envelope (protocol §12.1,
/// ADR-006 §36 Q5). Namespace for the digest computed over a sorted set of
/// sub-evaluation digests under a profile's aggregation policy.
pub const AUTHORITY_AGGREGATE_DOMAIN: &str = "statechronicle.authority.aggregate.v0";

/// How the evaluations of a deployment's authority set are combined for a
/// transition (protocol §18.1 step 8, ADR-006 §36 Q5).
///
/// The default is [`AggregationPolicy::RequireAll`]: every member of the
/// deployment's authority set must allow the operation. Profiles may declare
/// [`AggregationPolicy::AnyOf`] to pass when at least one authority allows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum AggregationPolicy {
    /// Every evaluated authority member must allow the operation.
    RequireAll,
    /// At least one evaluated authority member must allow the operation.
    AnyOf,
}

/// BCS-serializable envelope for a multi-authority aggregate digest.
///
/// This is a private aggregation helper, never exposed on the wire: its BCS
/// bytes are hashed inside [`aggregate_evaluation_digest`] to produce the
/// canonical aggregate [`ContentDigest`]. It pins the aggregate domain, the
/// policy the digest was computed under, and the sorted, deduplicated
/// sub-evaluation digests so the digest is deterministic and order-independent
/// (ADR-006 §36 Q5).
#[doc(hidden)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct AuthorityAggregate {
    /// Namespace tag, always [`AUTHORITY_AGGREGATE_DOMAIN`].
    domain: String,
    /// The aggregation policy the sub-evaluations were combined under.
    policy: AggregationPolicy,
    /// Sorted, deduplicated digests of the member sub-evaluations.
    sub_digests: Vec<ContentDigest>,
}

/// Computes the canonical aggregate digest over a set of TrustGrant
/// sub-evaluation digests under `policy`.
///
/// The sub-digests are deduplicated and sorted by raw bytes (`BTreeSet`), so
/// the digest is deterministic and independent of the order in which the
/// deployment's authority members were evaluated. With a single unique digest
/// the identity rule applies: the sub-digest itself is returned, preserving
/// v0 single-evaluator bytes. Otherwise the BCS bytes of the
/// [`AuthorityAggregate`] envelope are hashed via
/// [`hash_bytes`](statechronicle_core::digest::hash_bytes). This function is
/// total and never panics (protocol §12.1, ADR-006 §36 Q5).
pub fn aggregate_evaluation_digest(
    policy: AggregationPolicy,
    sub_digests: &[ContentDigest],
) -> ContentDigest {
    let sorted: Vec<ContentDigest> = sub_digests
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    if sorted.len() == 1
        && let Some(single) = sorted.first()
    {
        return single.clone();
    }
    let aggregate = AuthorityAggregate {
        domain: String::from(AUTHORITY_AGGREGATE_DOMAIN),
        policy,
        sub_digests: sorted,
    };
    let bytes = bcs::to_bytes(&aggregate).unwrap_or_default();
    hash_bytes(&bytes)
}

/// The outcome of a TrustGrant authority evaluation, bound into an event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrustGrantOutcome {
    /// Digest of the evaluated evidence, computed by trustgrant's own codec
    /// and treated as an opaque reference by StateChronicle (ADR-003, §16.3).
    /// Stored as a canonical `sha256:` content digest; serde uses the same
    /// string form as the previous plain string.
    pub evaluation_digest: ContentDigest,
    /// The evaluation result: `allow` or `deny`.
    pub result: EvaluationResult,
    /// When the evaluation was performed (UTC).
    pub evaluated_at: DateTime<Utc>,
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;

    fn evaluated_at() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-07-14T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    #[test]
    fn authority_proof_serde_roundtrips_including_evaluated_at() {
        let proof = AuthorityProof {
            kind: String::from(TRUSTGRANT_EVALUATION_KIND),
            evaluation_digest: ContentDigest::new([7u8; 32]),
            result: EvaluationResult::Allow,
            evaluated_at: evaluated_at(),
        };
        let json = serde_json::to_string(&proof).unwrap();
        assert!(json.contains("evaluated_at"));
        let decoded: AuthorityProof = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, proof);
    }

    fn digest(fill: u8) -> ContentDigest {
        ContentDigest::new([fill; 32])
    }

    #[test]
    fn aggregate_known_answer_two_member() {
        // Deterministic BCS envelope over (domain, RequireAll, sorted
        // sub-digests) hashed with SHA-256. Hard-coded so any change to the
        // envelope (domain tag, field order, encoding) is caught here.
        let aggregate =
            aggregate_evaluation_digest(AggregationPolicy::RequireAll, &[digest(1), digest(2)]);
        assert_eq!(
            aggregate.as_str(),
            "sha256:8c5a3a2805e25a2233a7f02cf0ad31709b07be941241c1f0b77bb8ca6f4370b3"
        );
    }

    #[test]
    fn aggregate_is_order_independent_and_deduplicates() {
        let d1 = digest(7);
        let d2 = digest(9);
        let d3 = digest(11);
        let mut permutation = vec![d2.clone(), d1.clone(), d3.clone(), d1.clone(), d2, d3];
        let baseline = aggregate_evaluation_digest(AggregationPolicy::RequireAll, &permutation);
        // Rotate/shuffle the same multiset; the digest must not change.
        for _ in 0..6 {
            permutation.rotate_left(1);
            let rerun = aggregate_evaluation_digest(AggregationPolicy::RequireAll, &permutation);
            assert_eq!(rerun, baseline);
        }
        // A duplicate-only set is identical to the single-element set.
        let dupes = aggregate_evaluation_digest(
            AggregationPolicy::RequireAll,
            &[d1.clone(), d1.clone(), d1.clone()],
        );
        let single = aggregate_evaluation_digest(AggregationPolicy::RequireAll, &[d1]);
        assert_eq!(dupes, single);
    }

    #[test]
    fn aggregate_single_element_is_identity() {
        let d = digest(3);
        for policy in [AggregationPolicy::RequireAll, AggregationPolicy::AnyOf] {
            let aggregate = aggregate_evaluation_digest(policy, std::slice::from_ref(&d));
            assert_eq!(
                aggregate, d,
                "identity rule preserves single-evaluator bytes"
            );
        }
    }

    #[test]
    fn aggregate_policy_is_bound_into_digest() {
        let set = [digest(1), digest(2)];
        let require_all = aggregate_evaluation_digest(AggregationPolicy::RequireAll, &set);
        let any_of = aggregate_evaluation_digest(AggregationPolicy::AnyOf, &set);
        assert_ne!(require_all, any_of, "policy must participate in the digest");
    }

    #[test]
    fn aggregate_envelope_bcs_is_deterministic() {
        let envelope = AuthorityAggregate {
            domain: String::from(AUTHORITY_AGGREGATE_DOMAIN),
            policy: AggregationPolicy::RequireAll,
            sub_digests: vec![digest(1), digest(2)],
        };
        let first = bcs::to_bytes(&envelope).unwrap();
        let second = bcs::to_bytes(&envelope).unwrap();
        assert_eq!(first, second);
    }
}

/// Allow/deny result of a TrustGrant evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EvaluationResult {
    /// The actor is authorized for the operation on the resource in the scope.
    Allow,
    /// The actor is not authorized.
    Deny,
}

/// Canonical authority proof block embedded in events and proof bundles.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthorityProof {
    /// Registered kind string, e.g. `trustgrant.evaluation`.
    pub kind: String,
    /// Digest of the referenced authority evaluation or proof bundle, stored
    /// as a canonical `sha256:` content digest.
    pub evaluation_digest: ContentDigest,
    /// Evaluation result bound into the transition.
    pub result: EvaluationResult,
    /// When the evaluation was performed (UTC); lets an offline verifier check
    /// revocation freshness without resolving the digest — see §36 Q3 decision.
    pub evaluated_at: DateTime<Utc>,
}
