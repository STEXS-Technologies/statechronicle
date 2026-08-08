#![no_main]

use libfuzzer_sys::fuzz_target;

use statechronicle_accumulator::key::StateKey;
use statechronicle_accumulator::sparse_merkle::{StateAccumulator, StateUpdate};

// The sparse Merkle accumulator must never panic on arbitrary input, and its
// proof roundtrips must hold for any leaf set: every inserted key yields a
// valid inclusion proof, and any non-inserted probe key yields a valid
// non-membership proof (or a valid inclusion proof when it is present).
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
        assert!(StateAccumulator::verify_inclusion(&root, &proof));
    }

    // A probe derived from the input: if present it must verify as included,
    // otherwise its slot must verify as empty.
    let mut probe_key = [0u8; 32];
    for (i, byte) in data.iter().take(32).enumerate() {
        probe_key[i] = *byte;
    }
    let probe = StateKey::new(probe_key);
    match acc.prove_inclusion(&probe) {
        Some(proof) => {
            assert!(StateAccumulator::verify_inclusion(&root, &proof));
        }
        None => {
            let proof = acc.prove_non_membership(&probe).unwrap();
            assert!(StateAccumulator::verify_non_membership(&root, &proof));
        }
    }

    // Duplicate batches (same set, different order) must agree on the root.
    // The order-independence invariant holds for a well-formed `(key -> leaf)`
    // set: a batch that maps a key to two *different* digests is not a set (the
    // protocol's builders apply such batches in a meaningful order, later wins),
    // so the order-independence check is skipped for those.
    if well_defined_set(&updates) {
        let mut reversed_updates = updates.clone();
        reversed_updates.reverse();
        let mut other = StateAccumulator::empty();
        if let Ok(other_root) = other.insert_batch(&reversed_updates) {
            assert_eq!(root, other_root);
        }
    }
});

/// Returns `true` when every key in `updates` maps to exactly one digest, i.e.
/// the batch is a well-defined `(key -> leaf)` set whose root must be
/// independent of insertion order.
fn well_defined_set(updates: &[StateUpdate]) -> bool {
    use std::collections::BTreeMap;
    let mut seen: BTreeMap<StateKey, [u8; 32]> = BTreeMap::new();
    for update in updates {
        match seen.get(&update.key) {
            Some(prev) if *prev != update.state_digest => return false,
            _ => {
                seen.insert(update.key, update.state_digest);
            }
        }
    }
    true
}
