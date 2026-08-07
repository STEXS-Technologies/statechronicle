//! Proptest roundtrip properties for the sparse Merkle accumulator.
//!
//! Locks the ADR-005 correctness claims:
//! - `verify_inclusion ∘ prove_inclusion = identity` (verify always accepts a
//!   genuine proof),
//! - `verify_non_membership ∘ prove_non_membership = identity`,
//! - the root is a pure function of the `(key → digest)` set (insertion
//!   order independent), and
//! - a wrong digest can never verify against a genuine proof.

#![allow(clippy::panic, clippy::unwrap_used, clippy::indexing_slicing)]

use proptest::prelude::*;
use statechronicle_accumulator::key::StateKey;
use statechronicle_accumulator::sparse_merkle::{StateAccumulator, StateRoot, StateUpdate};

fn arb_key() -> impl Strategy<Value = StateKey> {
    prop::array::uniform32(any::<u8>()).prop_map(StateKey::new)
}

fn arb_update() -> impl Strategy<Value = StateUpdate> {
    (arb_key(), prop::array::uniform32(any::<u8>()))
        .prop_map(|(key, digest)| StateUpdate::new(key, digest))
}

fn arb_updates() -> impl Strategy<Value = Vec<StateUpdate>> {
    prop::collection::vec(arb_update(), 0..=24)
}

/// Returns a deterministic root for the (key → digest) set: insertion order
/// does not matter, so a canonical build is equivalent to any other.
fn canonical_root(updates: &[StateUpdate]) -> StateRoot {
    let mut entries: Vec<StateUpdate> = updates.to_vec();
    entries.sort_by_key(|a| a.key);
    entries.dedup_by_key(|u| u.key);
    let mut acc = StateAccumulator::empty();
    acc.insert_batch(&entries).unwrap();
    acc.root()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]
    #[test]
    fn inclusion_proofs_verify(updates in arb_updates()) {
        let mut acc = StateAccumulator::empty();
        acc.insert_batch(&updates).unwrap();
        let root = acc.root();
        // Every inserted key must produce a valid inclusion proof.
        for update in &updates {
            let proof = acc.prove_inclusion(&update.key).unwrap();
            prop_assert!(StateAccumulator::verify_inclusion(&root, &proof));
        }
    }

    #[test]
    fn non_membership_proofs_verify(updates in arb_updates()) {
        let mut acc = StateAccumulator::empty();
        acc.insert_batch(&updates).unwrap();
        let root = acc.root();
        // A fresh key (with negligible probability colliding) proves absent.
        let missing = StateKey::new([0x5au8; 32]);
        if acc.prove_inclusion(&missing).is_none() {
            let proof = acc.prove_non_membership(&missing).unwrap();
            prop_assert!(StateAccumulator::verify_non_membership(&root, &proof));
        }
    }

    #[test]
    fn root_is_independent_of_insertion_order(updates in arb_updates()) {
        let mut forward = StateAccumulator::empty();
        forward.insert_batch(&updates).unwrap();

        let mut reversed = StateAccumulator::empty();
        let mut rev = updates.clone();
        rev.reverse();
        reversed.insert_batch(&rev).unwrap();

        prop_assert_eq!(forward.root(), reversed.root());
        prop_assert_eq!(forward.root(), canonical_root(&updates));
    }

    #[test]
    fn wrong_digest_never_verifies(
        updates in arb_updates(),
        key in arb_key(),
        wrong_digest in prop::array::uniform32(any::<u8>()),
    ) {
        // Build a tree that provably contains `key` (or does not), then show
        // that a leaf built over `wrong_digest` cannot verify — unless the
        // wrong digest is exactly the stored one (astronomically unlikely) or
        // the tree is empty and the proof is an empty non-membership slot.
        let mut acc = StateAccumulator::empty();
        acc.insert_batch(&updates).unwrap();
        let root = acc.root();

        if acc.prove_inclusion(&key).is_some() {
            let mut proof = acc.prove_inclusion(&key).unwrap();
            let stored = updates
                .iter()
                .find(|u| u.key == key)
                .map_or([0u8; 32], |u| u.state_digest);
            prop_assume!(stored != wrong_digest);
            proof.leaf_hash = statechronicle_accumulator::sparse_merkle::leaf_hash(key, wrong_digest);
            prop_assert!(!StateAccumulator::verify_inclusion(&root, &proof));
        }
    }

    #[test]
    fn checkpoint_proofs_verify(tenant_count in 1usize..=16) {
        use statechronicle_accumulator::checkpoint::CheckpointRoot;
        use statechronicle_domain::tenant::TenantId;

        let mut pairs = Vec::new();
        for i in 0..tenant_count {
            let tenant = TenantId(format!("tenant:check_{i:04}"));
            let mut bytes = [0u8; 32];
            bytes[0] = i as u8;
            pairs.push((tenant, StateRoot::new(bytes)));
        }
        let checkpoint = CheckpointRoot::from_tenant_roots(&pairs).unwrap();
        for (tenant, _) in &pairs {
            let proof = checkpoint.prove_tenant_root(tenant).unwrap();
            prop_assert!(CheckpointRoot::verify_tenant_root(&checkpoint, &proof));
        }
    }
}
