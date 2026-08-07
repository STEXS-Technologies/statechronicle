#![no_main]

use libfuzzer_sys::fuzz_target;

use statechronicle_accumulator::key::StateKey;
use statechronicle_accumulator::sparse_merkle::{StateAccumulator, StateUpdate};
use statechronicle_core::digest::ContentDigest;
use statechronicle_proof::inclusion::{sparse_proof_from_inclusion, steps_from_sparse_proof};
use statechronicle_proof::verify::verify_sparse_merkle_v0;

// The sparse Merkle proof lane must never panic on arbitrary input, and its
// roundtrips must hold for any leaf set:
// - every inserted key's accumulator proof converts to a dense v0 wire proof
//   that verifies against the same root through the accumulator's own path
//   verifier (reused by the proof lane),
// - the dense wire roundtrip preserves the accumulator's level-tagged steps,
// - a tampered leaf hash can never verify against the same root.
fuzz_target!(|data: &[u8]| {
    // Derive a deterministic batch of (key, digest) updates from the input.
    let mut updates = Vec::new();
    for chunk in data.chunks(64).take(32) {
        let mut key = [0u8; 32];
        let mut digest = [0u8; 32];
        for (i, byte) in chunk.iter().take(32).enumerate() {
            key[i] = *byte;
        }
        for (i, byte) in chunk.iter().skip(32).take(32).enumerate() {
            digest[i] = *byte;
        }
        updates.push(StateUpdate::new(StateKey::new(key), digest));
    }

    let mut acc = StateAccumulator::empty();
    let Ok(root) = acc.insert_batch(&updates) else {
        return;
    };

    for update in &updates {
        let proof = acc.prove_inclusion(&update.key).unwrap();
        let sparse = sparse_proof_from_inclusion(&proof);

        // Genuine proofs verify through the dense wire form.
        assert!(verify_sparse_merkle_v0(&root, &update.key, &sparse).is_ok());

        // The dense roundtrip preserves the accumulator's step list.
        let steps = steps_from_sparse_proof(&sparse).unwrap();
        assert_eq!(steps, proof.steps);

        // A tampered leaf can never verify against the same root.
        let mut tampered = sparse.clone();
        tampered.leaf_hash = ContentDigest::new([0x5au8; 32]);
        assert!(verify_sparse_merkle_v0(&root, &update.key, &tampered).is_err());
    }
});
