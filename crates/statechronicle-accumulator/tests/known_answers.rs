//! Locked known-answer vectors for the ADR-005 accumulator encoding.
//!
//! These vectors pin the node/leaf encoding, the empty-subtree constants,
//! the key-derivation preimage, and the checkpoint composition. They are the
//! conformance artifact for non-Rust verifiers.

#![allow(clippy::panic, clippy::unwrap_used, clippy::indexing_slicing)]

use statechronicle_accumulator::checkpoint::CheckpointRoot;
use statechronicle_accumulator::key::StateKey;
use statechronicle_accumulator::sparse_merkle::{
    EMPTY_LEAF_HASH, StateAccumulator, StateRoot, StateUpdate, default_hash,
};
use statechronicle_domain::tenant::TenantId;

fn hex(bytes: &[u8; 32]) -> String {
    hex::encode(bytes)
}

#[test]
fn empty_leaf_hash_known_answer() {
    assert_eq!(
        hex(&EMPTY_LEAF_HASH),
        "3abddd594d884ff661d24666372d9fe669a1ec9e616401db6bd2edeb3c397143"
    );
}

#[test]
fn default_subtree_hashes_known_answers() {
    assert_eq!(
        hex(&default_hash(1)),
        "6461a6d00f23efd18b789a7cb4f6b3a437f0468a2268cf5a46043befaafc2f21"
    );
    assert_eq!(
        hex(&default_hash(2)),
        "3152aa7de5f1bab693be587943333797481648fe792eaa3b37a80de532f70473"
    );
    assert_eq!(
        hex(&default_hash(256)),
        "c810a0e262c2af1f3b6dbb71c26fe246b0b6b5a3e903759595d17601f0178311"
    );
}

#[test]
fn for_resource_key_known_answer() {
    let key = StateKey::for_resource("tenant:acme", "asset:sword_001");
    assert_eq!(
        hex(key.as_bytes()),
        "0421c524d1c882a9eaa2f91e189a126e6c1b0cd2f66fb53beee2016ee8ddd14a"
    );
}

#[test]
fn for_subject_held_key_known_answer() {
    let key = StateKey::for_subject_held(
        "tenant:acme",
        "asset:sword_001",
        "account:example:player_123",
    );
    assert_eq!(
        hex(key.as_bytes()),
        "f0d26f5ba7d7380ceff5e6f33c78ed1df06556499435a657dbb9f3f3b0d0907d"
    );
}

#[test]
fn checkpoint_two_tenant_root_known_answer() {
    let pairs = [
        (
            TenantId(String::from("tenant:alpha")),
            StateRoot::new([0xaau8; 32]),
        ),
        (
            TenantId(String::from("tenant:beta")),
            StateRoot::new([0xbbu8; 32]),
        ),
    ];
    let checkpoint = CheckpointRoot::from_tenant_roots(&pairs).unwrap();
    assert_eq!(
        hex(checkpoint.as_bytes()),
        "fe1c9e911c7a12302a09e57cd02cb218426ba5a734148f64a9748e691710ade6"
    );
}

#[test]
fn empty_accumulator_root_known_answer() {
    // The empty tenant tree root is default[256].
    let root = StateAccumulator::empty().root();
    assert_eq!(
        hex(root.as_bytes()),
        "c810a0e262c2af1f3b6dbb71c26fe246b0b6b5a3e903759595d17601f0178311"
    );
}

#[test]
fn single_update_root_and_inclusion_verify() {
    // A single leaf in a 256-level tree: root combines the leaf upward with
    // defaults. Lock the roundtrip, not an independent root value (no such
    // vector exists for multi-key SMT roots in ADR-005 v1.0).
    let key = StateKey::for_resource("tenant:acme", "asset:sword_001");
    let digest = [0x5au8; 32];
    let mut acc = StateAccumulator::empty();
    acc.insert_batch(&[StateUpdate::new(key, digest)]).unwrap();
    let root = acc.root();
    let proof = acc.prove_inclusion(&key).unwrap();
    assert!(StateAccumulator::verify_inclusion(&root, &proof));
}
