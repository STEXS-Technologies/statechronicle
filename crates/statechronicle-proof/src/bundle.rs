//! Proof bundle assembly (protocol §16.2).
//!
//! Combines inclusion, state, ownership, and authority proofs into a portable
//! [`ResourceStateProof`] envelope that can be verified independently of the
//! issuing server. The envelope pins the enclosing signed commit (via
//! [`CommitRef`], whose detached signature lets a verifier check the commit
//! signature against the bundle) and carries a dense v0 sparse Merkle proof
//! of the claimed state leaf.
//!
//! The builders are assembling functions: they enforce structural
//! consistency (tenant/resource/leaf agreement) and leave all cryptographic
//! checks to [`crate::verify`].

use chrono::{DateTime, Utc};

use statechronicle_accumulator::key::StateKey;
use statechronicle_accumulator::proof::{InclusionProof, NonMembershipProof};
use statechronicle_accumulator::sparse_merkle::{EMPTY_LEAF_HASH, leaf_hash};
use statechronicle_core::digest::{ContentDigest, hash_bytes};
use statechronicle_domain::authority::{
    AuthorityProof, EvaluationResult, TRUSTGRANT_EVALUATION_KIND,
};
use statechronicle_domain::commit::{Commit, ScopeKind};
use statechronicle_domain::ids::SnapshotId;
use statechronicle_domain::intent::Operation;
use statechronicle_domain::proof::{
    CommitRef, EventRef, NonMembershipProofBundle, ResourceStateProof, SparseMerkleProof,
};
use statechronicle_domain::resource::ResourceId;
use statechronicle_domain::signed::Signed;
use statechronicle_domain::state::StateProjection;
use statechronicle_domain::subject::SubjectId;
use statechronicle_domain::tenant::TenantId;

use crate::error::ProofError;
use crate::inclusion::{sparse_proof_from_inclusion, sparse_proof_from_non_membership};

/// Schema identifier for v0 snapshot proofs.
pub const SNAPSHOT_PROOF_SCHEMA: &str = "statechronicle.proof.snapshot.v0";

/// A portable snapshot proof (protocol §15).
///
/// Binds a snapshot's payload digest to the resource state proof of the
/// snapshot's enclosing commit, so authenticity and the pinned state root can
/// be checked together.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SnapshotProof {
    /// Schema identifier, always [`SNAPSHOT_PROOF_SCHEMA`] for v0.
    pub schema: String,
    /// The snapshot being proven.
    pub snapshot_id: SnapshotId,
    /// Canonical SHA-256 digest of the snapshot payload bytes.
    pub payload_digest: ContentDigest,
    /// The resource state proof pinning the snapshot's state root.
    pub state: ResourceStateProof,
}

impl SnapshotProof {
    /// Constructs a snapshot proof with the v0 schema identifier set.
    pub fn new(
        snapshot_id: SnapshotId,
        payload_digest: ContentDigest,
        state: ResourceStateProof,
    ) -> Self {
        Self {
            schema: String::from(SNAPSHOT_PROOF_SCHEMA),
            snapshot_id,
            payload_digest,
            state,
        }
    }
}

/// Assembles a v0 resource state proof for a resource projection.
///
/// The projection's `state` becomes the bundle's `claimed_state`, the signed
/// commit's `next_state_root` pins the state root, and the accumulator's
/// inclusion proof is converted into the dense v0 sparse Merkle wire form.
///
/// `key` is the state key of the proven leaf. It is derived by the caller from the
/// resource's state type (owner-based: [`StateKey::for_resource`];
/// subject-held: [`StateKey::for_subject_held`]).
///
/// # Errors
///
/// Returns [`ProofError::CommitScope`] when the enclosing commit is not
/// tenant-scoped, [`ProofError::LeafMismatch`] when the inclusion proof's
/// leaf does not commit the projection's `state_hash`, and
/// [`ProofError::Domain`] when a domain id cannot be constructed.
#[allow(clippy::too_many_arguments)]
pub fn build_state_proof(
    projection: &StateProjection,
    signed_commit: &Signed<Commit>,
    inclusion: &InclusionProof,
    latest_operation: &Operation,
    authority: Option<AuthorityProof>,
    key: StateKey,
) -> Result<ResourceStateProof, ProofError> {
    let expected_leaf = leaf_hash(key, *projection.state_hash.as_bytes());
    if inclusion.leaf_hash != expected_leaf {
        return Err(ProofError::LeafMismatch);
    }
    let commit = commit_ref(signed_commit)?;
    let sparse = sparse_proof_from_inclusion(inclusion);
    let latest_event = EventRef {
        event_id: projection.last_event_id.clone(),
        operation: latest_operation.clone(),
    };
    Ok(ResourceStateProof::new(
        projection.tenant_id.clone(),
        projection.resource_id.clone(),
        projection.state.clone(),
        commit,
        sparse,
        latest_event,
        authority,
    ))
}

/// Assembles an ownership proof: a state proof whose claimed state is
/// additionally bound to a subject.
///
/// Checks that the projection's claimed state carries an `owner` field equal
/// to `subject` before assembling the bundle (protocol §29 step 8).
///
/// # Errors
///
/// Returns every error of [`build_state_proof`], plus
/// [`ProofError::InvalidState`] when the claimed state has no string `owner`
/// field and [`ProofError::SubjectMismatch`] when the owner does not equal
/// `subject`.
#[allow(clippy::too_many_arguments)]
pub fn build_ownership_proof(
    projection: &StateProjection,
    signed_commit: &Signed<Commit>,
    inclusion: &InclusionProof,
    latest_operation: &Operation,
    subject: &SubjectId,
    authority: Option<AuthorityProof>,
    key: StateKey,
) -> Result<ResourceStateProof, ProofError> {
    let owner = owner_of(&projection.state)?;
    if owner != subject.0 {
        return Err(ProofError::SubjectMismatch {
            expected: String::from(subject.0.as_str()),
            actual: owner,
        });
    }
    build_state_proof(
        projection,
        signed_commit,
        inclusion,
        latest_operation,
        authority,
        key,
    )
}

/// Assembles an inclusion proof for a state leaf from an accumulator proof.
///
/// The returned [`SparseMerkleProof`] is the dense v0 wire form with
/// `kind = sparse_merkle_v0`.
pub fn build_inclusion_proof(inclusion: &InclusionProof) -> SparseMerkleProof {
    sparse_proof_from_inclusion(inclusion)
}

/// Assembles a non-membership proof bundle (protocol §16.2).
///
/// Binds an absent state key to the enclosing signed commit. The accumulator's
/// non-membership proof authenticates that the key's slot holds the empty-leaf
/// constant; the bundle carries that proof (converted to the dense v0 wire
/// form) with `claimed_key` set to the 32 raw key bytes as a digest.
///
/// # Errors
///
/// Returns [`ProofError::KeyMismatch`] when the non-membership proof's key
/// does not equal `key`, [`ProofError::NonMembershipLeafMismatch`] when the
/// proof's leaf is not the empty-leaf constant, and
/// [`ProofError::CommitScope`] when the enclosing commit is not tenant-scoped.
pub fn build_non_membership_proof(
    tenant: &TenantId,
    resource_id: &ResourceId,
    key: &StateKey,
    signed_commit: &Signed<Commit>,
    non_membership: &NonMembershipProof,
) -> Result<NonMembershipProofBundle, ProofError> {
    if non_membership.key != *key {
        return Err(ProofError::KeyMismatch {
            expected: format!("{key}"),
            actual: format!("{}", non_membership.key),
        });
    }
    if non_membership.leaf_hash != EMPTY_LEAF_HASH {
        return Err(ProofError::NonMembershipLeafMismatch);
    }
    let commit = commit_ref(signed_commit)?;
    let state_non_membership_proof = sparse_proof_from_non_membership(non_membership);
    Ok(NonMembershipProofBundle::new(
        tenant.clone(),
        resource_id.clone(),
        ContentDigest::new(*key.as_bytes()),
        commit,
        state_non_membership_proof,
    ))
}

/// Assembles a TrustGrant authority proof block (protocol §12.1).
///
/// Binds an evaluation outcome (`allow`/`deny`) to the opaque content digest
/// of the authority evaluation. The ledger never parses the evaluation's
/// internal structure; verifiers resolve the digest through their own
/// trustgrant integration (protocol §16.3, §29).
///
/// Since Phase 2, for multi-authority transitions `evaluation_digest` is the
/// **aggregate** digest over the sorted sub-evaluation digests (or the
/// sub-evaluation digest itself for a single-member set); the individual
/// sub-evaluations are not embedded in the bundle (ADR-006 §36 Q5, Q6).
///
/// `evaluated_at` records when the evaluation was performed so an offline
/// verifier can check revocation freshness without resolving the digest
/// (protocol §36 Q3 decision).
pub fn build_authority_proof(
    evaluation_digest: ContentDigest,
    result: EvaluationResult,
    evaluated_at: DateTime<Utc>,
) -> AuthorityProof {
    AuthorityProof {
        kind: String::from(TRUSTGRANT_EVALUATION_KIND),
        evaluation_digest,
        result,
        evaluated_at,
    }
}

/// Assembles a snapshot proof binding a snapshot payload to a state proof.
///
/// `payload_digest` is the canonical SHA-256 digest of the snapshot payload
/// bytes (protocol §15, §29 "malicious snapshots" defense).
pub fn build_snapshot_proof(
    snapshot_id: SnapshotId,
    payload: &[u8],
    state: ResourceStateProof,
) -> SnapshotProof {
    SnapshotProof::new(snapshot_id, hash_bytes(payload), state)
}

/// Returns the owner-based state key implied by a proof's tenant and resource.
///
/// For subject-held resources use [`derive_state_key`] instead, which reads
/// the `subject` field from the claimed state.
pub fn state_key_for_proof(proof: &ResourceStateProof) -> StateKey {
    StateKey::for_resource(&proof.tenant_id.0, &proof.resource_id.0)
}

/// Derives the state key implied by a proof's tenant, resource, and claimed
/// state (protocol §9 keying conventions, ADR-005 §2).
///
/// A claimed state carrying a string `subject` is subject-held; otherwise the
/// proof is resource-keyed.
///
/// # Errors
///
/// Returns [`ProofError::InvalidState`] when the claimed state's `subject` is
/// not a non-empty string.
pub fn derive_state_key(proof: &ResourceStateProof) -> Result<StateKey, ProofError> {
    let tenant = &proof.tenant_id.0;
    let resource = &proof.resource_id.0;
    if let Some(subject) = proof
        .claimed_state
        .get("subject")
        .and_then(|value| value.as_str())
    {
        if subject.is_empty() {
            return Err(ProofError::InvalidState(String::from(
                "claimed state `subject` is empty",
            )));
        }
        Ok(StateKey::for_subject_held(tenant, resource, subject))
    } else {
        Ok(StateKey::for_resource(tenant, resource))
    }
}

/// Reads the `owner` field of a claimed state payload.
///
/// # Errors
///
/// Returns [`ProofError::InvalidState`] when the state carries no `owner`
/// field or its `owner` is not a non-empty string.
pub fn owner_of(state: &serde_json::Value) -> Result<String, ProofError> {
    let owner = state
        .get("owner")
        .and_then(|value| value.as_str())
        .ok_or_else(|| {
            ProofError::InvalidState(String::from("state is missing a string `owner`"))
        })?;
    if owner.is_empty() {
        return Err(ProofError::InvalidState(String::from(
            "state `owner` is empty",
        )));
    }
    Ok(String::from(owner))
}

/// Extracts the signed commit reference embedded in proof bundles.
///
/// # Errors
///
/// Returns [`ProofError::CommitScope`] when the commit is not tenant-scoped.
pub fn commit_ref(signed_commit: &Signed<Commit>) -> Result<CommitRef, ProofError> {
    let body = &signed_commit.body;
    if body.scope.kind != ScopeKind::Tenant {
        return Err(ProofError::CommitScope(String::from(
            "proof bundles pin tenant-scoped commits; global checkpoint commits carry tenant roots, not resource state",
        )));
    }
    let _tenant = body.scope.tenant_id.as_ref().ok_or_else(|| {
        ProofError::CommitScope(String::from(
            "tenant-scoped commit is missing its tenant id",
        ))
    })?;
    Ok(CommitRef {
        commit_id: body.commit_id.clone(),
        sequence: body.sequence,
        state_root: body.next_state_root.clone(),
        signature: signed_commit.signature.clone(),
    })
}

/// Verifies that a proof's tenant and resource match the requested claim.
///
/// # Errors
///
/// Returns [`ProofError::TenantMismatch`] or [`ProofError::ResourceMismatch`]
/// when either identity differs.
pub fn check_claim(
    proof: &ResourceStateProof,
    tenant: &TenantId,
    resource: &ResourceId,
) -> Result<(), ProofError> {
    if proof.tenant_id != *tenant {
        return Err(ProofError::TenantMismatch {
            expected: String::from(tenant.0.as_str()),
            actual: String::from(proof.tenant_id.0.as_str()),
        });
    }
    if proof.resource_id != *resource {
        return Err(ProofError::ResourceMismatch {
            expected: String::from(resource.0.as_str()),
            actual: String::from(proof.resource_id.0.as_str()),
        });
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use chrono::{DateTime, Utc};
    use ed25519_dalek::SigningKey;
    use statechronicle_accumulator::sparse_merkle::{StateAccumulator, StateUpdate};
    use statechronicle_core::canonicalize::canonicalize_and_digest;
    use statechronicle_core::digest::hash_bytes;
    use statechronicle_domain::authority::{AggregationPolicy, aggregate_evaluation_digest};
    use statechronicle_domain::commit::{CommitScope, ProfileId};
    use statechronicle_domain::ids::{CommitId, EventId};
    use statechronicle_domain::state::StateProjection;
    use statechronicle_domain::state_type::StateType;

    const FIXED_SEED: [u8; 32] = [42u8; 32];

    fn tenant() -> TenantId {
        TenantId(String::from("acme.game.alpha"))
    }

    fn resource() -> ResourceId {
        ResourceId(String::from("asset:sword_001"))
    }

    fn owner() -> String {
        String::from("account:example:player_456")
    }

    fn executor() -> SubjectId {
        SubjectId(String::from("service:statechronicle.example.net"))
    }

    fn timestamp() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-07-14T00:00:02Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn key() -> SigningKey {
        SigningKey::from_bytes(&FIXED_SEED)
    }

    fn signed_commit() -> Signed<Commit> {
        let commit = Commit::new(
            CommitScope::tenant(tenant()),
            CommitId::new(String::from("cmt_01JZ8X5HN3C4PXG5A9FGEWQF5W")).unwrap(),
            None,
            918273,
            1,
            hash_bytes(b"event-root"),
            hash_bytes(b"previous-root"),
            hash_bytes(b"next-root"),
            timestamp(),
            executor(),
            ProfileId::new(String::from("statechronicle.profile.resource.v0")).unwrap(),
        );
        let canonical = statechronicle_core::canonicalize::canonicalize(&commit).unwrap();
        let signature = statechronicle_core::signature::sign(&canonical, &key());
        Signed::new(
            commit,
            statechronicle_domain::intent::SignatureBlock {
                alg: statechronicle_domain::intent::SignatureAlg::Ed25519,
                key_id: statechronicle_domain::intent::KeyId::new(String::from(
                    "did:key:z6Mk...#statechronicle-commit",
                ))
                .unwrap(),
                sig: signature,
            },
        )
    }

    fn projection(key: StateKey) -> StateProjection {
        let state = serde_json::json!({
            "owner": owner(),
            "status": "active",
            "version": 42,
        });
        let state_hash = canonicalize_and_digest(&state).unwrap();
        let mut acc = StateAccumulator::empty();
        acc.insert_batch(&[StateUpdate::new(key, *state_hash.as_bytes())])
            .unwrap();
        StateProjection {
            tenant_id: tenant(),
            resource_id: resource(),
            state_type: StateType::UniqueAsset,
            version: 42,
            last_event_id: EventId::new(String::from("evt_01JZ8X2XRE5ZYW5V9R7VDQBSH4")).unwrap(),
            last_commit_id: CommitId::new(String::from("cmt_01JZ8X5HN3C4PXG5A9FGEWQF5W")).unwrap(),
            state_hash,
            state,
        }
    }

    fn inclusion(key: StateKey) -> InclusionProof {
        let state = serde_json::json!({ "owner": owner(), "status": "active", "version": 42 });
        let state_hash = canonicalize_and_digest(&state).unwrap();
        let mut acc = StateAccumulator::empty();
        acc.insert_batch(&[StateUpdate::new(key, *state_hash.as_bytes())])
            .unwrap();
        acc.prove_inclusion(&key).unwrap()
    }

    #[test]
    fn build_state_proof_assembles_envelope() {
        let key = StateKey::for_resource(&tenant().0, &resource().0);
        let proof = build_state_proof(
            &projection(key),
            &signed_commit(),
            &inclusion(key),
            &Operation::new(String::from("asset.transfer")).unwrap(),
            None,
            key,
        )
        .unwrap();

        assert_eq!(proof.schema, "statechronicle.proof.resource_state.v0");
        assert_eq!(proof.tenant_id, tenant());
        assert_eq!(proof.resource_id, resource());
        assert_eq!(
            proof.state_inclusion_proof.kind,
            statechronicle_domain::proof::SPARSE_MERKLE_V0
        );
        assert_eq!(proof.commit.commit_id, signed_commit().body.commit_id);
        assert_eq!(proof.commit.sequence, 918273);
        assert_eq!(
            proof.commit.state_root,
            signed_commit().body.next_state_root
        );
        assert_eq!(
            proof.latest_event.event_id.as_str(),
            "evt_01JZ8X2XRE5ZYW5V9R7VDQBSH4"
        );
        assert_eq!(proof.latest_event.operation.as_str(), "asset.transfer");
    }

    #[test]
    fn build_state_proof_rejects_mismatched_leaf() {
        let key = StateKey::for_resource(&tenant().0, &resource().0);
        let mut projection = projection(key);
        projection.state_hash = hash_bytes(b"something-else");
        let error = build_state_proof(
            &projection,
            &signed_commit(),
            &inclusion(key),
            &Operation::new(String::from("asset.transfer")).unwrap(),
            None,
            key,
        )
        .unwrap_err();
        assert!(matches!(error, ProofError::LeafMismatch));
    }

    #[test]
    fn build_ownership_proof_checks_owner() {
        let key = StateKey::for_resource(&tenant().0, &resource().0);
        let good = SubjectId(String::from("account:example:player_456"));
        let proof = build_ownership_proof(
            &projection(key),
            &signed_commit(),
            &inclusion(key),
            &Operation::new(String::from("asset.transfer")).unwrap(),
            &good,
            None,
            key,
        )
        .unwrap();
        assert_eq!(proof.claimed_state["owner"], serde_json::json!(owner()));

        let bad = SubjectId(String::from("account:example:player_789"));
        let error = build_ownership_proof(
            &projection(key),
            &signed_commit(),
            &inclusion(key),
            &Operation::new(String::from("asset.transfer")).unwrap(),
            &bad,
            None,
            key,
        )
        .unwrap_err();
        assert!(matches!(error, ProofError::SubjectMismatch { .. }));
    }

    #[test]
    fn build_authority_and_snapshot_proofs() {
        let authority = build_authority_proof(
            hash_bytes(b"evaluation"),
            EvaluationResult::Allow,
            timestamp(),
        );
        assert_eq!(authority.kind, TRUSTGRANT_EVALUATION_KIND);
        assert_eq!(authority.result, EvaluationResult::Allow);

        let key = StateKey::for_resource(&tenant().0, &resource().0);
        let state = build_state_proof(
            &projection(key),
            &signed_commit(),
            &inclusion(key),
            &Operation::new(String::from("asset.transfer")).unwrap(),
            Some(authority.clone()),
            key,
        )
        .unwrap();
        assert_eq!(state.authority, Some(authority));

        let snapshot_id = SnapshotId::new(String::from("snp_01JZ8X9P4DC6YC4K1YZEJX45E2")).unwrap();
        let snapshot = build_snapshot_proof(snapshot_id.clone(), b"snapshot payload", state);
        assert_eq!(snapshot.schema, SNAPSHOT_PROOF_SCHEMA);
        assert_eq!(snapshot.snapshot_id, snapshot_id);
        assert_eq!(snapshot.payload_digest, hash_bytes(b"snapshot payload"));
    }

    #[test]
    fn aggregate_authority_proof_binds_into_resource_state_proof() {
        // A multi-authority aggregate digest (two distinct sub-evaluations)
        // binds through build_authority_proof into a ResourceStateProof.
        let aggregate = aggregate_evaluation_digest(
            AggregationPolicy::RequireAll,
            &[hash_bytes(b"sub-eval-a"), hash_bytes(b"sub-eval-b")],
        );
        let authority =
            build_authority_proof(aggregate.clone(), EvaluationResult::Allow, timestamp());

        let key = StateKey::for_resource(&tenant().0, &resource().0);
        let state = build_state_proof(
            &projection(key),
            &signed_commit(),
            &inclusion(key),
            &Operation::new(String::from("asset.transfer")).unwrap(),
            Some(authority.clone()),
            key,
        )
        .unwrap();
        assert_eq!(state.authority, Some(authority));
        assert_eq!(
            state.authority.unwrap().evaluation_digest,
            aggregate,
            "the bound proof carries the aggregate digest"
        );
    }

    #[test]
    fn derive_state_key_distinguishes_subject_held() {
        let mut proof = ResourceStateProof::new(
            tenant(),
            resource(),
            serde_json::json!({ "owner": owner(), "status": "active" }),
            commit_ref(&signed_commit()).unwrap(),
            SparseMerkleProof::new(Vec::new(), hash_bytes(b"leaf")),
            EventRef {
                event_id: EventId::new(String::from("evt_01JZ8X2XRE5ZYW5V9R7VDQBSH4")).unwrap(),
                operation: Operation::new(String::from("asset.transfer")).unwrap(),
            },
            None,
        );
        assert_eq!(
            derive_state_key(&proof).unwrap(),
            StateKey::for_resource(&tenant().0, &resource().0)
        );

        proof.claimed_state = serde_json::json!({
            "subject": "account:example:player_123",
            "balance": "100",
        });
        assert_eq!(
            derive_state_key(&proof).unwrap(),
            StateKey::for_subject_held(&tenant().0, &resource().0, "account:example:player_123")
        );

        proof.claimed_state = serde_json::json!({ "subject": "" });
        assert!(matches!(
            derive_state_key(&proof),
            Err(ProofError::InvalidState(_))
        ));
    }

    #[test]
    fn check_claim_validates_identities() {
        let key = StateKey::for_resource(&tenant().0, &resource().0);
        let proof = build_state_proof(
            &projection(key),
            &signed_commit(),
            &inclusion(key),
            &Operation::new(String::from("asset.transfer")).unwrap(),
            None,
            key,
        )
        .unwrap();
        assert!(check_claim(&proof, &tenant(), &resource()).is_ok());
        assert!(matches!(
            check_claim(
                &proof,
                &TenantId(String::from("acme.game.beta")),
                &resource()
            ),
            Err(ProofError::TenantMismatch { .. })
        ));
        assert!(matches!(
            check_claim(
                &proof,
                &tenant(),
                &ResourceId(String::from("asset:shield_002"))
            ),
            Err(ProofError::ResourceMismatch { .. })
        ));
    }

    #[test]
    fn commit_ref_rejects_global_checkpoint() {
        let global = Commit::new(
            CommitScope::global_checkpoint(),
            CommitId::new(String::from("cmt_checkpoint_001")).unwrap(),
            None,
            1,
            1,
            hash_bytes(b"event-root"),
            hash_bytes(b"previous-root"),
            hash_bytes(b"next-root"),
            timestamp(),
            executor(),
            ProfileId::new(String::from("statechronicle.profile.resource.v0")).unwrap(),
        );
        let canonical = statechronicle_core::canonicalize::canonicalize(&global).unwrap();
        let signature = statechronicle_core::signature::sign(&canonical, &key());
        let signed = Signed::new(
            global,
            statechronicle_domain::intent::SignatureBlock {
                alg: statechronicle_domain::intent::SignatureAlg::Ed25519,
                key_id: statechronicle_domain::intent::KeyId::new(String::from(
                    "did:key:z6Mk...#statechronicle-commit",
                ))
                .unwrap(),
                sig: signature,
            },
        );
        assert!(matches!(
            commit_ref(&signed),
            Err(ProofError::CommitScope(_))
        ));
    }

    fn present_key() -> StateKey {
        StateKey::for_resource(&tenant().0, &resource().0)
    }

    fn absent_key() -> StateKey {
        StateKey::new([0xabu8; 32])
    }

    fn non_membership_for(absent: StateKey) -> NonMembershipProof {
        let present = present_key();
        let mut acc = StateAccumulator::empty();
        acc.insert_batch(&[StateUpdate::new(present, *hash_bytes(b"state").as_bytes())])
            .unwrap();
        acc.prove_non_membership(&absent).unwrap()
    }

    #[test]
    fn build_non_membership_proof_assembles_bundle() {
        let absent = absent_key();
        let non_membership = non_membership_for(absent);
        let bundle = build_non_membership_proof(
            &tenant(),
            &resource(),
            &absent,
            &signed_commit(),
            &non_membership,
        )
        .unwrap();

        assert_eq!(bundle.schema, "statechronicle.proof.non_membership.v0");
        assert_eq!(bundle.tenant_id, tenant());
        assert_eq!(bundle.resource_id, resource());
        assert_eq!(bundle.claimed_key.as_bytes(), absent.as_bytes());
        assert_eq!(
            bundle.state_non_membership_proof.kind,
            statechronicle_domain::proof::SPARSE_MERKLE_V0
        );
        assert_eq!(bundle.state_non_membership_proof.path.len(), 256);
        assert_eq!(
            bundle.state_non_membership_proof.leaf_hash.as_bytes(),
            &statechronicle_accumulator::sparse_merkle::EMPTY_LEAF_HASH
        );
        assert_eq!(bundle.commit.commit_id, signed_commit().body.commit_id);
        assert_eq!(bundle.commit.sequence, 918273);
    }

    #[test]
    fn build_non_membership_proof_rejects_wrong_key() {
        let proof_for = non_membership_for(absent_key());
        let other = StateKey::new([0xbbu8; 32]);
        assert!(matches!(
            build_non_membership_proof(
                &tenant(),
                &resource(),
                &other,
                &signed_commit(),
                &proof_for,
            ),
            Err(ProofError::KeyMismatch { .. })
        ));
    }

    #[test]
    fn build_non_membership_proof_rejects_non_empty_leaf() {
        let absent = absent_key();
        let mut non_membership = non_membership_for(absent);
        non_membership.leaf_hash = [0x5au8; 32];
        assert!(matches!(
            build_non_membership_proof(
                &tenant(),
                &resource(),
                &absent,
                &signed_commit(),
                &non_membership,
            ),
            Err(ProofError::NonMembershipLeafMismatch)
        ));
    }

    #[test]
    fn build_non_membership_proof_rejects_global_checkpoint() {
        let global = Commit::new(
            CommitScope::global_checkpoint(),
            CommitId::new(String::from("cmt_checkpoint_001")).unwrap(),
            None,
            1,
            1,
            hash_bytes(b"event-root"),
            hash_bytes(b"previous-root"),
            hash_bytes(b"next-root"),
            timestamp(),
            executor(),
            ProfileId::new(String::from("statechronicle.profile.resource.v0")).unwrap(),
        );
        let canonical = statechronicle_core::canonicalize::canonicalize(&global).unwrap();
        let signature = statechronicle_core::signature::sign(&canonical, &key());
        let signed = Signed::new(
            global,
            statechronicle_domain::intent::SignatureBlock {
                alg: statechronicle_domain::intent::SignatureAlg::Ed25519,
                key_id: statechronicle_domain::intent::KeyId::new(String::from(
                    "did:key:z6Mk...#statechronicle-commit",
                ))
                .unwrap(),
                sig: signature,
            },
        );
        let absent = absent_key();
        let non_membership = non_membership_for(absent);
        assert!(matches!(
            build_non_membership_proof(&tenant(), &resource(), &absent, &signed, &non_membership),
            Err(ProofError::CommitScope(_))
        ));
    }
}
