//! Integration tests for the sparse Merkle accumulator and checkpoint root.

#![allow(clippy::panic, clippy::unwrap_used, clippy::indexing_slicing)]

use statechronicle_accumulator::checkpoint::CheckpointRoot;
use statechronicle_accumulator::key::StateKey;
use statechronicle_accumulator::sparse_merkle::{StateAccumulator, StateRoot, StateUpdate};
use statechronicle_domain::tenant::TenantId;

#[test]
fn insertion_order_does_not_affect_root() {
    let updates: Vec<StateUpdate> = (0..64)
        .map(|i| {
            let resource = format!("asset:item_{i:04}");
            let mut digest = [0u8; 32];
            digest[0] = i;
            StateUpdate::new(StateKey::for_resource("tenant:acme", &resource), digest)
        })
        .collect();

    let mut forward = StateAccumulator::empty();
    forward.insert_batch(&updates).unwrap();

    let mut reversed = StateAccumulator::empty();
    let mut rev = updates.clone();
    rev.reverse();
    reversed.insert_batch(&rev).unwrap();

    let mut interleaved = StateAccumulator::empty();
    for (i, update) in updates.iter().enumerate() {
        let _ = i;
        interleaved
            .insert_batch(std::slice::from_ref(update))
            .unwrap();
    }

    assert_eq!(forward.root(), reversed.root());
    assert_eq!(forward.root(), interleaved.root());
}

#[test]
fn inclusion_and_non_membership_verify_against_root() {
    let mut acc = StateAccumulator::empty();
    let mut updates = Vec::new();
    for i in 0..40 {
        updates.push(StateUpdate::new(
            StateKey::for_resource("tenant:acme", &format!("asset:item_{i:04}")),
            [i as u8; 32],
        ));
    }
    acc.insert_batch(&updates).unwrap();
    let root = acc.root();

    for update in &updates {
        let proof = acc.prove_inclusion(&update.key).unwrap();
        assert!(
            StateAccumulator::verify_inclusion(&root, &proof),
            "inclusion proof must verify for {:?}",
            update.key
        );
    }

    let missing = StateKey::for_resource("tenant:acme", "asset:absent_0000");
    assert!(acc.prove_inclusion(&missing).is_none());
    let non_membership = acc.prove_non_membership(&missing).unwrap();
    assert!(StateAccumulator::verify_non_membership(
        &root,
        &non_membership
    ));
}

#[test]
fn subject_held_and_owner_keys_coexist() {
    let owner = StateKey::for_resource("tenant:acme", "asset:sword_001");
    let held =
        StateKey::for_subject_held("tenant:acme", "asset:sword_001", "account:stexs:player_123");
    let mut acc = StateAccumulator::empty();
    acc.insert_batch(&[
        StateUpdate::new(owner, [0x11u8; 32]),
        StateUpdate::new(held, [0x22u8; 32]),
    ])
    .unwrap();
    let root = acc.root();
    for key in [owner, held] {
        let proof = acc.prove_inclusion(&key).unwrap();
        assert!(StateAccumulator::verify_inclusion(&root, &proof));
    }
}

#[test]
fn checkpoint_tenant_root_proofs_verify_for_many_tenants() {
    let mut pairs = Vec::new();
    for i in 0..50 {
        let tenant = TenantId(format!("tenant:org_{i:03}"));
        let mut bytes = [0u8; 32];
        bytes[0] = i;
        pairs.push((tenant, StateRoot::new(bytes)));
    }
    let checkpoint = CheckpointRoot::from_tenant_roots(&pairs).unwrap();

    for (tenant, _) in &pairs {
        let proof = checkpoint.prove_tenant_root(tenant).unwrap();
        assert!(CheckpointRoot::verify_tenant_root(&checkpoint, &proof));
    }

    let absent = TenantId(String::from("tenant:nope"));
    assert!(checkpoint.prove_tenant_root(&absent).is_none());
}

#[test]
fn checkpoint_sorting_makes_root_order_independent() {
    let alpha = TenantId(String::from("tenant:alpha"));
    let beta = TenantId(String::from("tenant:beta"));
    let gamma = TenantId(String::from("tenant:gamma"));
    let ra = StateRoot::new([0xaau8; 32]);
    let rb = StateRoot::new([0xbbu8; 32]);
    let rc = StateRoot::new([0xccu8; 32]);

    let first = CheckpointRoot::from_tenant_roots(&[
        (alpha.clone(), ra),
        (beta.clone(), rb),
        (gamma.clone(), rc),
    ])
    .unwrap();
    let second =
        CheckpointRoot::from_tenant_roots(&[(gamma, rc), (alpha, ra), (beta, rb)]).unwrap();

    assert_eq!(first.as_bytes(), second.as_bytes());
}

#[test]
fn checkpoint_single_tenant_has_empty_step_list() {
    let alpha = TenantId(String::from("tenant:alpha"));
    let root = StateRoot::new([0xaau8; 32]);
    let checkpoint = CheckpointRoot::from_tenant_roots(&[(alpha.clone(), root)]).unwrap();
    let proof = checkpoint.prove_tenant_root(&alpha).unwrap();
    assert!(proof.steps.is_empty());
    assert!(CheckpointRoot::verify_tenant_root(&checkpoint, &proof));
}
