//! Proptest roundtrip properties for the proof lane.
//!
//! Locks the correctness claims of the sparse Merkle proof path (protocol
//! §16.2–§16.3):
//! - every genuine inclusion proof converts to a dense v0 wire proof and
//!   verifies back against the same root (`verify_proof` accepts),
//! - the dense wire roundtrip preserves the accumulator's level-tagged steps,
//! - a genuine proof never verifies against a different root,
//! - a tampered claimed state never verifies (claimed-state gate),
//! - a tampered leaf hash never verifies (inclusion gate).

#![allow(
    clippy::panic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::redundant_clone,
    clippy::bool_assert_comparison,
    clippy::result_large_err
)]

use proptest::prelude::*;

use statechronicle_accumulator::key::StateKey;
use statechronicle_accumulator::proof::InclusionProof;
use statechronicle_accumulator::sparse_merkle::{StateAccumulator, StateUpdate};

use statechronicle_core::canonicalize::canonicalize_and_digest;
use statechronicle_core::digest::{ContentDigest, hash_bytes};
use statechronicle_core::signature::Signature;

use statechronicle_domain::ids::{CommitId, EventId};
use statechronicle_domain::intent::{KeyId, Operation, SignatureAlg, SignatureBlock};
use statechronicle_domain::proof::{
    CommitRef, EventRef, NonMembershipProofBundle, ResourceStateProof,
};
use statechronicle_domain::resource::ResourceId;
use statechronicle_domain::tenant::TenantId;

use statechronicle_proof::error::ProofError;
use statechronicle_proof::inclusion::{
    sparse_proof_from_inclusion, sparse_proof_from_non_membership, steps_from_sparse_proof,
};
use statechronicle_proof::verify::{verify_non_membership, verify_proof, verify_sparse_merkle_v0};

fn arb_key() -> impl Strategy<Value = StateKey> {
    prop::array::uniform32(any::<u8>()).prop_map(StateKey::new)
}

/// Generates a BCS-encodable claimed state: a JSON object of short ASCII
/// string fields (no floats, since the protocol bans floating-point state).
fn arb_state() -> impl Strategy<Value = serde_json::Value> {
    fn arb_ascii() -> impl Strategy<Value = String> {
        prop::collection::vec(prop::char::range('a', 'z'), 1..=12)
            .prop_map(|chars| chars.into_iter().collect::<String>())
    }
    prop::collection::btree_map(arb_ascii(), arb_ascii(), 1..=6).prop_map(|entries| {
        let mut object = serde_json::Map::new();
        for (key, value) in entries {
            object.insert(key, serde_json::Value::String(value));
        }
        serde_json::Value::Object(object)
    })
}

/// Builds a bundle whose claimed state is `state`, committed at `key` in the
/// accumulator that produced `inclusion`.
fn sample_proof(
    state: serde_json::Value,
    inclusion: &InclusionProof,
    root: &statechronicle_accumulator::sparse_merkle::StateRoot,
) -> ResourceStateProof {
    ResourceStateProof::new(
        TenantId(String::from("acme.game.alpha")),
        ResourceId(String::from("asset:sword_001")),
        state,
        CommitRef {
            commit_id: CommitId::new(String::from("cmt_01JZ8X5HN3C4PXG5A9FGEWQF5W")).unwrap(),
            sequence: 1,
            state_root: ContentDigest::new(*root.as_bytes()),
            signature: SignatureBlock {
                alg: SignatureAlg::Ed25519,
                key_id: KeyId::new(String::from("did:key:z6Mk...#key-1")).unwrap(),
                sig: Signature::from_bytes([0u8; 64]),
            },
        },
        sparse_proof_from_inclusion(inclusion),
        EventRef {
            event_id: EventId::new(String::from("evt_01JZ8X2XRE5ZYW5V9R7VDQBSH4")).unwrap(),
            operation: Operation::new(String::from("asset.transfer")).unwrap(),
        },
        None,
    )
}

/// Wraps an absent-key non-membership proof into a portable bundle.
fn non_membership_bundle(
    non_membership: &statechronicle_accumulator::proof::NonMembershipProof,
    root: &statechronicle_accumulator::sparse_merkle::StateRoot,
) -> NonMembershipProofBundle {
    NonMembershipProofBundle::new(
        TenantId(String::from("acme.game.alpha")),
        ResourceId(String::from("asset:sword_001")),
        ContentDigest::new(*non_membership.key.as_bytes()),
        CommitRef {
            commit_id: CommitId::new(String::from("cmt_01JZ8X5HN3C4PXG5A9FGEWQF5W")).unwrap(),
            sequence: 1,
            state_root: ContentDigest::new(*root.as_bytes()),
            signature: SignatureBlock {
                alg: SignatureAlg::Ed25519,
                key_id: KeyId::new(String::from("did:key:z6Mk...#key-1")).unwrap(),
                sig: Signature::from_bytes([0u8; 64]),
            },
        },
        sparse_proof_from_non_membership(non_membership),
    )
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(32))]

    #[test]
    fn genuine_state_proofs_verify(key in arb_key(), state in arb_state()) {
        let state_digest = canonicalize_and_digest(&state).unwrap();
        let mut acc = StateAccumulator::empty();
        acc.insert_batch(&[StateUpdate::new(key, *state_digest.as_bytes())]).unwrap();
        let root = acc.root();
        let inclusion = acc.prove_inclusion(&key).unwrap();

        let proof = sample_proof(state, &inclusion, &root);
        prop_assert!(verify_proof(&proof, &root, &key).is_ok());
        prop_assert!(verify_sparse_merkle_v0(&root, &key, &proof.state_inclusion_proof).is_ok());
    }

    #[test]
    fn dense_wire_roundtrip_preserves_steps(key in arb_key(), digest in prop::array::uniform32(any::<u8>())) {
        let mut acc = StateAccumulator::empty();
        acc.insert_batch(&[StateUpdate::new(key, digest)]).unwrap();
        let inclusion = acc.prove_inclusion(&key).unwrap();

        let sparse = sparse_proof_from_inclusion(&inclusion);
        prop_assert_eq!(steps_from_sparse_proof(&sparse).unwrap(), inclusion.steps);
        prop_assert_eq!(sparse.leaf_hash.as_bytes(), &inclusion.leaf_hash);
    }

    #[test]
    fn genuine_proof_rejects_wrong_root(key in arb_key(), state in arb_state()) {
        let state_digest = canonicalize_and_digest(&state).unwrap();
        let mut acc = StateAccumulator::empty();
        acc.insert_batch(&[StateUpdate::new(key, *state_digest.as_bytes())]).unwrap();
        let root = acc.root();
        let inclusion = acc.prove_inclusion(&key).unwrap();

        let proof = sample_proof(state, &inclusion, &root);
        let wrong_root = statechronicle_accumulator::sparse_merkle::StateRoot::new([0x5au8; 32]);
        prop_assert!(matches!(
            verify_proof(&proof, &wrong_root, &key),
            Err(ProofError::InclusionMismatch)
        ));
    }

    #[test]
    fn tampered_state_never_verifies(key in arb_key(), state in arb_state(), wrong in arb_state()) {
        prop_assume!(state != wrong);
        let state_digest = canonicalize_and_digest(&state).unwrap();
        let mut acc = StateAccumulator::empty();
        acc.insert_batch(&[StateUpdate::new(key, *state_digest.as_bytes())]).unwrap();
        let root = acc.root();
        let inclusion = acc.prove_inclusion(&key).unwrap();

        let mut proof = sample_proof(state, &inclusion, &root);
        proof.claimed_state = wrong;
        prop_assert!(matches!(
            verify_proof(&proof, &root, &key),
            Err(ProofError::ClaimedStateMismatch)
        ));
    }

    #[test]
    fn tampered_leaf_never_verifies(key in arb_key(), state in arb_state()) {
        let state_digest = canonicalize_and_digest(&state).unwrap();
        let mut acc = StateAccumulator::empty();
        acc.insert_batch(&[StateUpdate::new(key, *state_digest.as_bytes())]).unwrap();
        let root = acc.root();
        let inclusion = acc.prove_inclusion(&key).unwrap();

        let mut proof = sample_proof(state, &inclusion, &root);
        proof.state_inclusion_proof.leaf_hash = hash_bytes(b"not-the-leaf");
        prop_assert!(matches!(
            verify_proof(&proof, &root, &key),
            Err(ProofError::InclusionMismatch)
        ));
    }

    #[test]
    fn leaf_commits_canonical_state_digest(key in arb_key(), state in arb_state()) {
        let state_digest = canonicalize_and_digest(&state).unwrap();
        let mut acc = StateAccumulator::empty();
        acc.insert_batch(&[StateUpdate::new(key, *state_digest.as_bytes())]).unwrap();
        let inclusion = acc.prove_inclusion(&key).unwrap();

        let expected = statechronicle_accumulator::sparse_merkle::leaf_hash(
            key,
            *state_digest.as_bytes(),
        );
        prop_assert_eq!(inclusion.leaf_hash, expected);
    }

    #[test]
    fn non_membership_roundtrip_and_verify(key in arb_key(), probe in arb_key(), digest in prop::array::uniform32(any::<u8>())) {
        let mut acc = StateAccumulator::empty();
        acc.insert_batch(&[StateUpdate::new(key, digest)]).unwrap();
        let root = acc.root();

        // Only a genuinely absent probe yields a non-membership proof.
        if let Some(non_membership) = acc.prove_non_membership(&probe) {
            let sparse = sparse_proof_from_non_membership(&non_membership);
            prop_assert_eq!(sparse.path.len(), 256);
            prop_assert_eq!(sparse.leaf_hash.as_bytes(), &statechronicle_accumulator::sparse_merkle::EMPTY_LEAF_HASH);
            // The dense wire roundtrip preserves the accumulator's steps.
            prop_assert_eq!(steps_from_sparse_proof(&sparse).unwrap(), non_membership.steps.clone());

            let bundle = non_membership_bundle(&non_membership, &root);
            prop_assert!(verify_non_membership(&bundle, &root, &probe).is_ok());
        }
    }

    #[test]
    fn non_membership_deterministic(key in arb_key(), probe in arb_key(), digest in prop::array::uniform32(any::<u8>())) {
        let mut acc = StateAccumulator::empty();
        acc.insert_batch(&[StateUpdate::new(key, digest)]).unwrap();
        let root = acc.root();

        if let Some(non_membership) = acc.prove_non_membership(&probe) {
            let first = non_membership_bundle(&non_membership, &root);
            let second = non_membership_bundle(&non_membership, &root);
            prop_assert_eq!(
                &first.state_non_membership_proof,
                &second.state_non_membership_proof
            );
            prop_assert_eq!(first, second);
        }
    }
}
