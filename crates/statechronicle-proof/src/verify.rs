//! Verification algorithm (protocol §29, §16.3).
//!
//! Deterministic verification of proof bundles against roots and trust
//! anchors. The verifiers are pure: they recompute every commitment from the
//! proof and the supplied context and compare fail-closed, with no storage or
//! transport access.
//!
//! The sparse Merkle path check reuses the accumulator's own path verifier
//! (`[`StateAccumulator::verify_inclusion`]`), so the proof lane and the
//! accumulator can never disagree about what a genuine inclusion proof looks
//! like. Commit signatures are checked with `statechronicle_core`'s Ed25519
//! strict verifier (`ZIP-215` malleability checks) over the BCS canonical
//! commit body bytes (ADR-004 §2, §5).

use ed25519_dalek::VerifyingKey;
use statechronicle_accumulator::key::StateKey;
use statechronicle_accumulator::proof::{InclusionProof, NonMembershipProof};
use statechronicle_accumulator::sparse_merkle::{
    EMPTY_LEAF_HASH, StateAccumulator, StateRoot, TREE_DEPTH, leaf_hash,
};
use statechronicle_core::canonicalize::{canonicalize, canonicalize_and_digest};
use statechronicle_core::signature::verify as verify_signature;
use statechronicle_domain::commit::Commit;
use statechronicle_domain::proof::{
    NON_MEMBERSHIP_PROOF_SCHEMA, NonMembershipProofBundle, RESOURCE_STATE_PROOF_SCHEMA,
    ResourceStateProof, SPARSE_MERKLE_V0,
};
use statechronicle_domain::signed::Signed;
use statechronicle_domain::tenant::TenantId;

use crate::error::ProofError;
use crate::inclusion::steps_from_sparse_proof;

/// Verifies a signed commit's detached signature over its BCS canonical body
/// bytes (protocol §16.3 step 2, ADR-004 §2).
///
/// Uses `statechronicle_core`'s Ed25519 strict verification (ZIP-215
/// malleability checks), so a malformed, weak-key, or malleable signature is
/// rejected. The signature covers only `bcs::to_bytes(&body)`, never a
/// `signature` field.
///
/// # Errors
///
/// Returns [`ProofError::Core`] when the body cannot be BCS canonicalized and
/// [`ProofError::CommitSignature`] when the signature fails strict
/// verification.
pub fn verify_commit_signature_with_key(
    signed: &Signed<Commit>,
    verifying_key: &VerifyingKey,
) -> Result<(), ProofError> {
    let canonical = canonicalize(&signed.body)?;
    verify_signature(&canonical, verifying_key, &signed.signature.sig).map_err(|err| {
        ProofError::CommitSignature(format!(
            "commit signature failed strict Ed25519 verification: {err}"
        ))
    })
}

/// Verifies an accumulator inclusion proof against a state root.
///
/// Pure delegate to the accumulator's own path verifier: the single source
/// of truth for what a genuine inclusion proof looks like.
pub fn verify_inclusion(root: &StateRoot, proof: &InclusionProof) -> bool {
    StateAccumulator::verify_inclusion(root, proof)
}

/// Verifies a v0 sparse Merkle proof against a state root.
///
/// Rebuilds the level-tagged steps from the dense wire path and reuses the
/// accumulator's path verifier.
///
/// # Errors
///
/// Returns [`ProofError::UnsupportedKind`] for a non-v0 proof kind,
/// [`ProofError::InvalidPathLength`] for a malformed dense path, and
/// [`ProofError::InclusionMismatch`] when the recomputed root does not equal
/// `root`.
pub fn verify_sparse_merkle_v0(
    root: &StateRoot,
    key: &StateKey,
    sparse: &statechronicle_domain::proof::SparseMerkleProof,
) -> Result<(), ProofError> {
    if sparse.kind != SPARSE_MERKLE_V0 {
        return Err(ProofError::UnsupportedKind(sparse.kind.clone()));
    }
    let steps = steps_from_sparse_proof(sparse)?;
    let inclusion = InclusionProof {
        key: *key,
        leaf_hash: *sparse.leaf_hash.as_bytes(),
        steps,
    };
    if StateAccumulator::verify_inclusion(root, &inclusion) {
        Ok(())
    } else {
        Err(ProofError::InclusionMismatch)
    }
}

/// Verifies that a proof's claimed state hashes to the included leaf
/// (protocol §16.3 step 6, §29 step 7).
///
/// Recomputes the leaf `H(0x11 || key || state_digest)` from the claimed
/// state's canonical digest and compares it with the bundle's leaf hash.
///
/// # Errors
///
/// Returns [`ProofError::Core`] when the claimed state cannot be BCS
/// canonicalized and [`ProofError::ClaimedStateMismatch`] when the recomputed
/// leaf does not equal the bundled leaf hash.
pub fn verify_claimed_state(proof: &ResourceStateProof, key: &StateKey) -> Result<(), ProofError> {
    let state_digest = canonicalize_and_digest(&proof.claimed_state)?;
    let expected_leaf = leaf_hash(*key, *state_digest.as_bytes());
    if expected_leaf == *proof.state_inclusion_proof.leaf_hash.as_bytes() {
        Ok(())
    } else {
        Err(ProofError::ClaimedStateMismatch)
    }
}

/// Verifies a proof bundle's cryptographic core against a state root
/// (protocol §29 steps 1, 6, 7).
///
/// Checks the schema identifier, verifies the sparse Merkle inclusion proof
/// against `root`, and verifies that the claimed state hashes to the included
/// leaf. Commit signature and commit-membership checks are performed by
/// [`verify_bundle`], which needs the enclosing signed commit.
///
/// # Errors
///
/// Returns [`ProofError::UnsupportedSchema`], [`ProofError::UnsupportedKind`],
/// [`ProofError::InvalidPathLength`], [`ProofError::InclusionMismatch`],
/// [`ProofError::Core`], or [`ProofError::ClaimedStateMismatch`].
pub fn verify_proof(
    proof: &ResourceStateProof,
    root: &StateRoot,
    key: &StateKey,
) -> Result<(), ProofError> {
    if proof.schema != RESOURCE_STATE_PROOF_SCHEMA {
        return Err(ProofError::UnsupportedSchema(proof.schema.clone()));
    }
    verify_sparse_merkle_v0(root, key, &proof.state_inclusion_proof)?;
    verify_claimed_state(proof, key)
}

/// Verifies that a proof's claimed state names `expected_subject` as the
/// owner (protocol §29 step 8).
///
/// # Errors
///
/// Returns [`ProofError::InvalidState`] when the claimed state carries no
/// string `owner` field and [`ProofError::SubjectMismatch`] when the owner
/// does not equal `expected_subject`.
pub fn verify_ownership(
    proof: &ResourceStateProof,
    expected_subject: &str,
) -> Result<(), ProofError> {
    let owner = crate::bundle::owner_of(&proof.claimed_state)?;
    if owner == expected_subject {
        Ok(())
    } else {
        Err(ProofError::SubjectMismatch {
            expected: String::from(expected_subject),
            actual: owner,
        })
    }
}

/// Verifies the commit envelope shared by every proof bundle
/// (protocol §16.3 steps 2–4, §29 steps 3–5).
///
/// Checks, in order, that the enclosing commit is tenant-scoped and matches
/// the proof's tenant, that the bundle's commit reference (`commit_id`,
/// `sequence`, `state_root`) matches the signed commit body, and that the
/// detached commit signature verifies under `verifying_key`.
///
/// # Errors
///
/// Returns [`ProofError::CommitScope`], [`ProofError::TenantMismatch`],
/// [`ProofError::CommitRefMismatch`], or [`ProofError::CommitSignature`] in
/// the order above.
fn verify_commit_envelope(
    tenant_id: &TenantId,
    commit: &statechronicle_domain::proof::CommitRef,
    signed: &Signed<Commit>,
    verifying_key: &VerifyingKey,
) -> Result<(), ProofError> {
    let commit_tenant = signed.body.scope.tenant_id.as_ref().ok_or_else(|| {
        ProofError::CommitScope(String::from(
            "proof bundles pin tenant-scoped commits; global checkpoint commits carry tenant roots, not resource state",
        ))
    })?;
    if *commit_tenant != *tenant_id {
        return Err(ProofError::TenantMismatch {
            expected: String::from(commit_tenant.0.as_str()),
            actual: String::from(tenant_id.0.as_str()),
        });
    }

    if commit.commit_id != signed.body.commit_id {
        return Err(ProofError::CommitRefMismatch {
            expected: String::from(signed.body.commit_id.as_str()),
            actual: String::from(commit.commit_id.as_str()),
        });
    }
    if commit.sequence != signed.body.sequence {
        return Err(ProofError::CommitRefMismatch {
            expected: signed.body.sequence.to_string(),
            actual: commit.sequence.to_string(),
        });
    }
    if commit.state_root != signed.body.next_state_root {
        return Err(ProofError::CommitRefMismatch {
            expected: String::from(signed.body.next_state_root.as_str()),
            actual: String::from(commit.state_root.as_str()),
        });
    }

    verify_commit_signature_with_key(signed, verifying_key)
}

/// Verifies a proof bundle against its enclosing signed commit
/// (protocol §16.3 steps 1–7, §29 steps 2–7).
///
/// The full fail-closed pipeline:
///
/// 1. schema identifier check (step 1),
/// 2. tenant scope check against the enclosing commit (step 3),
/// 3. commit reference check: the bundle's `commit_id`, `sequence`, and
///    `state_root` must match the signed commit body (step 4),
/// 4. commit signature check via [`verify_commit_signature_with_key`]
///    (step 2),
/// 5. sparse Merkle inclusion against the commit's `next_state_root`
///    (step 6),
/// 6. claimed-state hash check (step 7).
///
/// # Errors
///
/// Returns every [`ProofError`] variant in fail-closed order above.
pub fn verify_bundle(
    proof: &ResourceStateProof,
    signed: &Signed<Commit>,
    verifying_key: &VerifyingKey,
    key: &StateKey,
) -> Result<(), ProofError> {
    if proof.schema != RESOURCE_STATE_PROOF_SCHEMA {
        return Err(ProofError::UnsupportedSchema(proof.schema.clone()));
    }

    verify_commit_envelope(&proof.tenant_id, &proof.commit, signed, verifying_key)?;

    let root = StateRoot::new(*signed.body.next_state_root.as_bytes());
    verify_sparse_merkle_v0(&root, key, &proof.state_inclusion_proof)?;
    verify_claimed_state(proof, key)
}

/// Verifies a non-membership proof bundle against a state root
/// (protocol §16.3, §29 "verifying absence").
///
/// The fail-closed pipeline for absence:
///
/// 1. schema identifier check,
/// 2. claimed-key check: the bundle's `claimed_key` must equal `key`,
/// 3. sparse Merkle proof kind check (`SPARSE_MERKLE_V0`),
/// 4. dense path length check ([`TREE_DEPTH`]),
/// 5. **empty-leaf assertion**: the proof's leaf must be the empty-leaf
///    constant: the load-bearing gate that makes the bundle a proof of
///    absence rather than presence (the accumulator's own
///    `verify_non_membership` does not assert the empty leaf, so this is
///    required),
/// 6. path verification: rebuild the level-tagged steps and delegate to
///    [`StateAccumulator::verify_non_membership`] against `root`.
///
/// # Errors
///
/// Returns [`ProofError::UnsupportedSchema`], [`ProofError::KeyMismatch`],
/// [`ProofError::UnsupportedKind`], [`ProofError::InvalidPathLength`],
/// [`ProofError::NonMembershipLeafMismatch`], or
/// [`ProofError::InclusionMismatch`] in fail-closed order above.
pub fn verify_non_membership(
    bundle: &NonMembershipProofBundle,
    root: &StateRoot,
    key: &StateKey,
) -> Result<(), ProofError> {
    if bundle.schema != NON_MEMBERSHIP_PROOF_SCHEMA {
        return Err(ProofError::UnsupportedSchema(bundle.schema.clone()));
    }
    if bundle.claimed_key.as_bytes() != key.as_bytes() {
        return Err(ProofError::KeyMismatch {
            expected: format!("{key}"),
            // Render the claimed key's raw 32 bytes as lowercase hex, matching
            // `expected` (StateKey Display) instead of the `sha256:` prefix
            // form, so both sides of the message read symmetrically.
            actual: format!("{}", StateKey::new(*bundle.claimed_key.as_bytes())),
        });
    }
    let sparse = &bundle.state_non_membership_proof;
    if sparse.kind != SPARSE_MERKLE_V0 {
        return Err(ProofError::UnsupportedKind(sparse.kind.clone()));
    }
    if sparse.path.len() != TREE_DEPTH {
        return Err(ProofError::InvalidPathLength {
            expected: TREE_DEPTH,
            actual: sparse.path.len(),
        });
    }
    if sparse.leaf_hash.as_bytes() != &EMPTY_LEAF_HASH {
        return Err(ProofError::NonMembershipLeafMismatch);
    }

    let steps = steps_from_sparse_proof(sparse)?;
    let proof = NonMembershipProof {
        key: *key,
        leaf_hash: *sparse.leaf_hash.as_bytes(),
        steps,
    };
    if StateAccumulator::verify_non_membership(root, &proof) {
        Ok(())
    } else {
        Err(ProofError::InclusionMismatch)
    }
}

/// Verifies a non-membership proof bundle against its enclosing signed commit
/// (protocol §16.3, §29 "verifying absence").
///
/// Mirrors [`verify_bundle`] for the non-membership bundle: schema check,
/// then the shared commit envelope (tenant scope, commit reference, commit
/// signature), then the absence-specific core
/// ([`verify_non_membership`]) against the commit's `next_state_root`.
///
/// # Errors
///
/// Returns every [`ProofError`] variant in fail-closed order above.
pub fn verify_non_membership_bundle(
    bundle: &NonMembershipProofBundle,
    signed: &Signed<Commit>,
    verifying_key: &VerifyingKey,
    key: &StateKey,
) -> Result<(), ProofError> {
    if bundle.schema != NON_MEMBERSHIP_PROOF_SCHEMA {
        return Err(ProofError::UnsupportedSchema(bundle.schema.clone()));
    }

    verify_commit_envelope(&bundle.tenant_id, &bundle.commit, signed, verifying_key)?;

    let root = StateRoot::new(*signed.body.next_state_root.as_bytes());
    verify_non_membership(bundle, &root, key)
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::redundant_clone,
    clippy::shadow_unrelated
)]
mod tests {
    use super::*;
    use chrono::{DateTime, Utc};
    use ed25519_dalek::SigningKey;
    use statechronicle_accumulator::sparse_merkle::{StateAccumulator, StateUpdate};
    use statechronicle_core::digest::hash_bytes;
    use statechronicle_domain::commit::{CommitScope, ProfileId};
    use statechronicle_domain::ids::{CommitId, EventId};
    use statechronicle_domain::intent::{KeyId, Operation, SignatureAlg, SignatureBlock};
    use statechronicle_domain::proof::{
        CommitRef, EventRef, NonMembershipProofBundle, SparseMerkleProof,
    };
    use statechronicle_domain::subject::SubjectId;
    use statechronicle_domain::tenant::TenantId;

    const FIXED_SEED: [u8; 32] = [42u8; 32];

    fn tenant() -> TenantId {
        TenantId(String::from("stexs.game.alpha"))
    }

    fn resource() -> String {
        String::from("asset:sword_001")
    }

    fn executor() -> SubjectId {
        SubjectId(String::from("service:statechronicle.stexs.net"))
    }

    fn timestamp() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-07-14T00:00:02Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn fixed_key() -> SigningKey {
        SigningKey::from_bytes(&FIXED_SEED)
    }

    fn key_id() -> KeyId {
        KeyId::new(String::from("did:key:z6Mk...#statechronicle-commit")).unwrap()
    }

    fn signed_commit(next_root: statechronicle_core::digest::ContentDigest) -> Signed<Commit> {
        let commit = Commit::new(
            CommitScope::tenant(tenant()),
            CommitId::new(String::from("cmt_01JZ8X5HN3C4PXG5A9FGEWQF5W")).unwrap(),
            None,
            918273,
            1,
            hash_bytes(b"event-root"),
            hash_bytes(b"previous-root"),
            next_root,
            timestamp(),
            executor(),
            ProfileId::new(String::from("statechronicle.profile.resource.v0")).unwrap(),
        );
        let canonical = canonicalize(&commit).unwrap();
        let signature = statechronicle_core::signature::sign(&canonical, &fixed_key());
        Signed::new(
            commit,
            SignatureBlock {
                alg: SignatureAlg::Ed25519,
                key_id: key_id(),
                sig: signature,
            },
        )
    }

    /// Builds an accumulator, inserts the claimed state digest at `key`, and
    /// returns the root plus a genuine inclusion proof.
    fn tree_with(key: StateKey, digest: [u8; 32]) -> (StateRoot, InclusionProof) {
        let mut acc = StateAccumulator::empty();
        acc.insert_batch(&[StateUpdate::new(key, digest)]).unwrap();
        let root = acc.root();
        let proof = acc.prove_inclusion(&key).unwrap();
        (root, proof)
    }

    fn claimed_state() -> serde_json::Value {
        serde_json::json!({
            "owner": "account:stexs:player_456",
            "status": "active",
            "version": 42,
        })
    }

    fn proof_for(key: StateKey, claimed: serde_json::Value) -> ResourceStateProof {
        let state_digest = canonicalize_and_digest(&claimed).unwrap();
        let (root, inclusion) = tree_with(key, *state_digest.as_bytes());
        let _ = root;
        let signed = signed_commit(statechronicle_core::digest::ContentDigest::new(
            *root.as_bytes(),
        ));
        ResourceStateProof::new(
            tenant(),
            statechronicle_domain::resource::ResourceId(resource()),
            claimed,
            CommitRef {
                commit_id: signed.body.commit_id.clone(),
                sequence: signed.body.sequence,
                state_root: signed.body.next_state_root.clone(),
                signature: signed.signature.clone(),
            },
            crate::inclusion::sparse_proof_from_inclusion(&inclusion),
            EventRef {
                event_id: EventId::new(String::from("evt_01JZ8X2XRE5ZYW5V9R7VDQBSH4")).unwrap(),
                operation: Operation::new(String::from("asset.transfer")).unwrap(),
            },
            None,
        )
    }

    #[test]
    fn verify_commit_signature_accepts_genuine_and_rejects_forged() {
        let signed = signed_commit(hash_bytes(b"next-root"));
        assert!(verify_commit_signature_with_key(&signed, &fixed_key().verifying_key()).is_ok());

        let other = SigningKey::from_bytes(&[7u8; 32]);
        assert!(verify_commit_signature_with_key(&signed, &other.verifying_key()).is_err());

        let mut tampered = signed.clone();
        tampered.body.sequence = tampered.body.sequence.wrapping_add(1);
        assert!(verify_commit_signature_with_key(&tampered, &fixed_key().verifying_key()).is_err());
    }

    #[test]
    fn verify_inclusion_delegates_to_accumulator() {
        let key = StateKey::for_resource(&tenant().0, &resource());
        let digest = [0xabu8; 32];
        let (root, proof) = tree_with(key, digest);
        assert!(verify_inclusion(&root, &proof));
        assert!(StateAccumulator::verify_inclusion(&root, &proof));
    }

    #[test]
    fn verify_proof_accepts_genuine_bundle() {
        let key = StateKey::for_resource(&tenant().0, &resource());
        let claimed = claimed_state();
        let state_digest = canonicalize_and_digest(&claimed).unwrap();
        let (root, _) = tree_with(key, *state_digest.as_bytes());
        let proof = proof_for(key, claimed);
        assert!(verify_proof(&proof, &root, &key).is_ok());
    }

    #[test]
    fn verify_proof_rejects_tampered_claimed_state() {
        let key = StateKey::for_resource(&tenant().0, &resource());
        let claimed = claimed_state();
        let state_digest = canonicalize_and_digest(&claimed).unwrap();
        let (root, _) = tree_with(key, *state_digest.as_bytes());
        let mut proof = proof_for(key, claimed);
        proof.claimed_state = serde_json::json!({
            "owner": "account:stexs:player_789",
            "status": "active",
            "version": 42,
        });
        assert!(matches!(
            verify_proof(&proof, &root, &key),
            Err(ProofError::ClaimedStateMismatch)
        ));
    }

    #[test]
    fn verify_proof_rejects_tampered_leaf() {
        let key = StateKey::for_resource(&tenant().0, &resource());
        let claimed = claimed_state();
        let state_digest = canonicalize_and_digest(&claimed).unwrap();
        let (root, _) = tree_with(key, *state_digest.as_bytes());
        let mut proof = proof_for(key, claimed);
        proof.state_inclusion_proof.leaf_hash = hash_bytes(b"wrong-leaf");
        assert!(matches!(
            verify_proof(&proof, &root, &key),
            Err(ProofError::InclusionMismatch)
        ));
    }

    #[test]
    fn verify_proof_rejects_wrong_root() {
        let key = StateKey::for_resource(&tenant().0, &resource());
        let proof = proof_for(key, claimed_state());
        let wrong_root = StateRoot::new([0x5au8; 32]);
        assert!(matches!(
            verify_proof(&proof, &wrong_root, &key),
            Err(ProofError::InclusionMismatch)
        ));
    }

    #[test]
    fn verify_proof_rejects_unknown_schema_and_kind() {
        let key = StateKey::for_resource(&tenant().0, &resource());
        let claimed = claimed_state();
        let state_digest = canonicalize_and_digest(&claimed).unwrap();
        let (root, _) = tree_with(key, *state_digest.as_bytes());

        let mut proof = proof_for(key, claimed);
        proof.schema = String::from("statechronicle.proof.resource_state.v9");
        assert!(matches!(
            verify_proof(&proof, &root, &key),
            Err(ProofError::UnsupportedSchema(_))
        ));

        let mut proof = proof_for(key, claimed_state());
        proof.state_inclusion_proof.kind = String::from("jellyfish_v1");
        assert!(matches!(
            verify_proof(&proof, &root, &key),
            Err(ProofError::UnsupportedKind(_))
        ));
    }

    #[test]
    fn verify_claimed_state_requires_leaf_commitment() {
        let key = StateKey::for_resource(&tenant().0, &resource());
        let claimed = claimed_state();
        let state_digest = canonicalize_and_digest(&claimed).unwrap();
        let (root, _) = tree_with(key, *state_digest.as_bytes());
        let _ = root;
        let proof = proof_for(key, claimed);
        assert!(verify_claimed_state(&proof, &key).is_ok());
    }

    #[test]
    fn verify_ownership_matches_expected_subject() {
        let key = StateKey::for_resource(&tenant().0, &resource());
        let proof = proof_for(key, claimed_state());
        assert!(verify_ownership(&proof, "account:stexs:player_456").is_ok());
        assert!(matches!(
            verify_ownership(&proof, "account:stexs:player_789"),
            Err(ProofError::SubjectMismatch { .. })
        ));

        let mut no_owner = proof_for(key, serde_json::json!({ "status": "active" }));
        let _ = &mut no_owner;
        let missing = ResourceStateProof::new(
            tenant(),
            statechronicle_domain::resource::ResourceId(resource()),
            serde_json::json!({ "status": "active" }),
            proof.commit.clone(),
            SparseMerkleProof::new(Vec::new(), hash_bytes(b"leaf")),
            proof.latest_event.clone(),
            None,
        );
        assert!(matches!(
            verify_ownership(&missing, "account:stexs:player_456"),
            Err(ProofError::InvalidState(_))
        ));
    }

    #[test]
    fn verify_bundle_accepts_genuine_proof() {
        let key = StateKey::for_resource(&tenant().0, &resource());
        let claimed = claimed_state();
        let state_digest = canonicalize_and_digest(&claimed).unwrap();
        let (root, _) = tree_with(key, *state_digest.as_bytes());
        let signed = signed_commit(statechronicle_core::digest::ContentDigest::new(
            *root.as_bytes(),
        ));
        let proof = proof_for(key, claimed);
        assert!(verify_bundle(&proof, &signed, &fixed_key().verifying_key(), &key).is_ok());
    }

    #[test]
    fn verify_bundle_rejects_tampered_signature() {
        let key = StateKey::for_resource(&tenant().0, &resource());
        let claimed = claimed_state();
        let state_digest = canonicalize_and_digest(&claimed).unwrap();
        let (root, _) = tree_with(key, *state_digest.as_bytes());
        let signed = signed_commit(statechronicle_core::digest::ContentDigest::new(
            *root.as_bytes(),
        ));
        let proof = proof_for(key, claimed);
        let other = SigningKey::from_bytes(&[7u8; 32]);
        assert!(verify_bundle(&proof, &signed, &other.verifying_key(), &key).is_err());
    }

    #[test]
    fn verify_bundle_rejects_commit_ref_mismatch() {
        let key = StateKey::for_resource(&tenant().0, &resource());
        let claimed = claimed_state();
        let state_digest = canonicalize_and_digest(&claimed).unwrap();
        let (root, _) = tree_with(key, *state_digest.as_bytes());
        let signed = signed_commit(statechronicle_core::digest::ContentDigest::new(
            *root.as_bytes(),
        ));
        let mut proof = proof_for(key, claimed);
        proof.commit.sequence = proof.commit.sequence.wrapping_add(1);
        assert!(matches!(
            verify_bundle(&proof, &signed, &fixed_key().verifying_key(), &key),
            Err(ProofError::CommitRefMismatch { .. })
        ));
    }

    #[test]
    fn verify_bundle_rejects_tenant_mismatch() {
        let key = StateKey::for_resource(&tenant().0, &resource());
        let claimed = claimed_state();
        let state_digest = canonicalize_and_digest(&claimed).unwrap();
        let (root, _) = tree_with(key, *state_digest.as_bytes());
        let signed = signed_commit(statechronicle_core::digest::ContentDigest::new(
            *root.as_bytes(),
        ));
        let mut proof = proof_for(key, claimed);
        proof.tenant_id = TenantId(String::from("stexs.game.beta"));
        assert!(matches!(
            verify_bundle(&proof, &signed, &fixed_key().verifying_key(), &key),
            Err(ProofError::TenantMismatch { .. })
        ));
    }

    fn absent_key() -> StateKey {
        StateKey::new([0xabu8; 32])
    }

    /// Builds an accumulator holding `present`, returns the signed commit
    /// pinning its root plus a genuine non-membership proof for `absent`.
    fn non_membership_fixture(
        present: StateKey,
        absent: StateKey,
    ) -> (NonMembershipProofBundle, Signed<Commit>, StateRoot) {
        let mut acc = StateAccumulator::empty();
        acc.insert_batch(&[StateUpdate::new(present, [0xabu8; 32])])
            .unwrap();
        let root = acc.root();
        let non_membership = acc.prove_non_membership(&absent).unwrap();
        let signed = signed_commit(statechronicle_core::digest::ContentDigest::new(
            *root.as_bytes(),
        ));
        let bundle = crate::bundle::build_non_membership_proof(
            &tenant(),
            &statechronicle_domain::resource::ResourceId(resource()),
            &absent,
            &signed,
            &non_membership,
        )
        .unwrap();
        (bundle, signed, root)
    }

    #[test]
    fn verify_non_membership_accepts_genuine_absence() {
        let present = StateKey::for_resource(&tenant().0, &resource());
        let absent = absent_key();
        let (bundle, signed, root) = non_membership_fixture(present, absent);
        assert!(verify_non_membership(&bundle, &root, &absent).is_ok());
        assert!(
            verify_non_membership_bundle(&bundle, &signed, &fixed_key().verifying_key(), &absent)
                .is_ok()
        );
    }

    #[test]
    fn verify_non_membership_rejects_wrong_schema() {
        let present = StateKey::for_resource(&tenant().0, &resource());
        let absent = absent_key();
        let (mut bundle, _signed, root) = non_membership_fixture(present, absent);
        bundle.schema = String::from("statechronicle.proof.non_membership.v9");
        assert!(matches!(
            verify_non_membership(&bundle, &root, &absent),
            Err(ProofError::UnsupportedSchema(_))
        ));
    }

    #[test]
    fn verify_non_membership_rejects_wrong_claimed_key() {
        let present = StateKey::for_resource(&tenant().0, &resource());
        let absent = absent_key();
        let (bundle, _signed, root) = non_membership_fixture(present, absent);
        let other = StateKey::new([0xbbu8; 32]);
        assert!(matches!(
            verify_non_membership(&bundle, &root, &other),
            Err(ProofError::KeyMismatch { .. })
        ));
    }

    #[test]
    fn verify_non_membership_rejects_wrong_kind() {
        let present = StateKey::for_resource(&tenant().0, &resource());
        let absent = absent_key();
        let (mut bundle, _signed, root) = non_membership_fixture(present, absent);
        bundle.state_non_membership_proof.kind = String::from("jellyfish_v1");
        assert!(matches!(
            verify_non_membership(&bundle, &root, &absent),
            Err(ProofError::UnsupportedKind(_))
        ));
    }

    #[test]
    fn verify_non_membership_rejects_wrong_path_length() {
        let present = StateKey::for_resource(&tenant().0, &resource());
        let absent = absent_key();
        let (mut bundle, _signed, root) = non_membership_fixture(present, absent);
        bundle.state_non_membership_proof.path.truncate(2);
        assert!(matches!(
            verify_non_membership(&bundle, &root, &absent),
            Err(ProofError::InvalidPathLength {
                expected: 256,
                actual: 2
            })
        ));
    }

    #[test]
    fn verify_non_membership_rejects_occupied_slot_fail_closed() {
        // The accumulator's `verify_non_membership` does NOT assert the empty
        // leaf, so a genuine inclusion proof of a PRESENT key smuggled in as a
        // "non-membership" bundle (leaf != EMPTY_LEAF_HASH) must be rejected
        // by the bundle verifier's empty-leaf gate.
        let present = StateKey::for_resource(&tenant().0, &resource());
        let (root, inclusion) = tree_with(present, [0xabu8; 32]);
        let sparse = crate::inclusion::sparse_proof_from_inclusion(&inclusion);
        let signed = signed_commit(statechronicle_core::digest::ContentDigest::new(
            *root.as_bytes(),
        ));
        let bundle = NonMembershipProofBundle::new(
            tenant(),
            statechronicle_domain::resource::ResourceId(resource()),
            statechronicle_core::digest::ContentDigest::new(*present.as_bytes()),
            CommitRef {
                commit_id: signed.body.commit_id.clone(),
                sequence: signed.body.sequence,
                state_root: signed.body.next_state_root.clone(),
                signature: signed.signature.clone(),
            },
            sparse,
        );
        assert_ne!(
            bundle.state_non_membership_proof.leaf_hash.as_bytes(),
            &statechronicle_accumulator::sparse_merkle::EMPTY_LEAF_HASH
        );
        assert!(matches!(
            verify_non_membership(&bundle, &root, &present),
            Err(ProofError::NonMembershipLeafMismatch)
        ));
    }

    #[test]
    fn verify_non_membership_rejects_wrong_root() {
        let present = StateKey::for_resource(&tenant().0, &resource());
        let absent = absent_key();
        let (bundle, _signed, _root) = non_membership_fixture(present, absent);
        let wrong_root = StateRoot::new([0x5au8; 32]);
        assert!(matches!(
            verify_non_membership(&bundle, &wrong_root, &absent),
            Err(ProofError::InclusionMismatch)
        ));
    }

    #[test]
    fn verify_non_membership_bundle_rejects_tenant_mismatch() {
        let present = StateKey::for_resource(&tenant().0, &resource());
        let absent = absent_key();
        let (mut bundle, signed, _root) = non_membership_fixture(present, absent);
        bundle.tenant_id = TenantId(String::from("stexs.game.beta"));
        assert!(matches!(
            verify_non_membership_bundle(&bundle, &signed, &fixed_key().verifying_key(), &absent),
            Err(ProofError::TenantMismatch { .. })
        ));
    }

    #[test]
    fn verify_non_membership_bundle_rejects_commit_ref_mismatch() {
        let present = StateKey::for_resource(&tenant().0, &resource());
        let absent = absent_key();
        let (mut bundle, signed, _root) = non_membership_fixture(present, absent);
        bundle.commit.sequence = bundle.commit.sequence.wrapping_add(1);
        assert!(matches!(
            verify_non_membership_bundle(&bundle, &signed, &fixed_key().verifying_key(), &absent),
            Err(ProofError::CommitRefMismatch { .. })
        ));
    }

    #[test]
    fn verify_non_membership_bundle_rejects_tampered_commit_body() {
        let present = StateKey::for_resource(&tenant().0, &resource());
        let absent = absent_key();
        let (bundle, mut signed, _root) = non_membership_fixture(present, absent);
        signed.body.created_at = signed
            .body
            .created_at
            .checked_add_signed(chrono::Duration::seconds(1))
            .unwrap();
        assert!(matches!(
            verify_non_membership_bundle(&bundle, &signed, &fixed_key().verifying_key(), &absent),
            Err(ProofError::CommitSignature(_))
        ));
    }

    #[test]
    fn verify_non_membership_bundle_rejects_wrong_verifying_key() {
        let present = StateKey::for_resource(&tenant().0, &resource());
        let absent = absent_key();
        let (bundle, signed, _root) = non_membership_fixture(present, absent);
        let other = SigningKey::from_bytes(&[7u8; 32]);
        assert!(
            verify_non_membership_bundle(&bundle, &signed, &other.verifying_key(), &absent)
                .is_err()
        );
    }
}
