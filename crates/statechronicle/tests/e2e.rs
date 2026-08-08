//! End-to-end integration test: the FULL workflow wired through the REAL
//! cross-crate pipeline of the umbrella crate.
//!
//! `submit → parse+validate (intent) → execute (executor, via in-memory port
//! fakes) → form+sign commit (commit) → state root (accumulator) → build proof
//! (proof) → verify end-to-end`.
//!
//! This is the test the review found missing: the umbrella's `smoke.rs` is
//! compile-only, and the per-crate integration tests exercise each crate in
//! isolation. This lane runs one `asset.mint → asset.transfer → asset.lock`
//! lifecycle through the real crates and asserts (a) the commit's state root is
//! the pure function of the emitted events, (b) a genuine proof bundle verifies
//! against the signed commit, (c) tampering one event's state payload fails
//! closed, and (d) a non-membership proof for an absent resource verifies.

#![allow(
    clippy::panic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::type_complexity
)]

mod common;

use std::collections::BTreeMap;

use serde_json::{Value, json};

use statechronicle::accumulator::key::StateKey;
use statechronicle::accumulator::sparse_merkle::{StateAccumulator, StateRoot};
use statechronicle::commit::batch::CommitBatch;
use statechronicle::commit::builder::CommitBuilder;
use statechronicle::commit::error::CommitError;
use statechronicle::commit::roots::{compute_state_root, state_root_updates};
use statechronicle::commit::sign::{sign_commit, verify_commit};
use statechronicle::core::canonicalize::canonicalize_and_digest;
use statechronicle::core::digest::ContentDigest;
use statechronicle::domain::commit::{CommitScope, ProfileId};
use statechronicle::domain::event::Event;
use statechronicle::domain::ids::CommitId;
use statechronicle::domain::intent::{INTENT_SCHEMA, Operation};
use statechronicle::domain::resource::ResourceId;
use statechronicle::domain::signed::Signed;
use statechronicle::domain::state::StateProjection;
use statechronicle::domain::state_type::StateType;
use statechronicle::proof::bundle::{
    build_non_membership_proof, build_state_proof, derive_state_key,
};
use statechronicle::proof::verify::{
    verify_bundle, verify_non_membership_bundle, verify_ownership,
};

use common::{
    Harness, executor_subject, fixed_key, fixed_timestamp_placeholder, intent_verifier, key_id,
    sample_authority, tenant, validated_intent,
};

const ALICE: &str = "account:example:player_123";
const BOB: &str = "account:example:player_456";
const RESOURCE: &str = "asset:sword_001";

/// The snapshot of the full pipeline produced by a single lifecycle run.
struct Lifecycle {
    events: Vec<Event>,
    signed: Signed<statechronicle::domain::commit::Commit>,
    accumulator: StateAccumulator,
    key: StateKey,
    projection: StateProjection,
    final_owner: String,
}

/// Builds a canonical raw-intent JSON payload.
fn payload(
    operation: &str,
    intent_id: &str,
    actor: &str,
    resource: &str,
    expected_version: u64,
    inputs: &BTreeMap<String, Value>,
) -> Value {
    json!({
        "schema": INTENT_SCHEMA,
        "tenant_id": "acme.game.alpha",
        "intent_id": intent_id,
        "operation": operation,
        "actor": actor,
        "resource_id": resource,
        "state_type": "unique_asset",
        "expected_version": expected_version,
        "inputs": inputs,
        "created_at": "2026-07-14T00:00:00Z",
        "expires_at": "2026-07-14T00:05:00Z",
        "nonce": "b64u:AAME",
    })
}

/// Builds an input map from entries.
fn inputs(entries: &[(&str, Value)]) -> BTreeMap<String, Value> {
    entries
        .iter()
        .map(|(key, value)| (String::from(*key), value.clone()))
        .collect()
}

fn profile() -> ProfileId {
    ProfileId::new(String::from("statechronicle.profile.resource.v0")).unwrap()
}

fn commit_id() -> Result<CommitId, CommitError> {
    CommitId::new(String::from("cmt_00000000000000000001")).map_err(CommitError::from)
}

fn batch_from(events: &[Event]) -> CommitBatch {
    let mut batch = CommitBatch::new(CommitScope::tenant(tenant()));
    for event in events {
        batch.add_event(event.clone()).unwrap();
    }
    batch
}

/// Runs the `asset.mint → asset.transfer → asset.lock` lifecycle through the
/// REAL executor with in-memory port fakes, forms + signs the commit, verifies
/// the state-root determinism, and returns the pieces needed for proof building.
async fn run_lifecycle() -> Lifecycle {
    let harness = Harness::with_verifier(intent_verifier());

    // §18.1: mint (version 0 -> 1), no prior state, to_owner = ALICE.
    let mint_intent = validated_intent(
        &payload(
            "asset.mint",
            "int_mint_001",
            ALICE,
            RESOURCE,
            0,
            &inputs(&[("to_owner", json!(ALICE))]),
        ),
        None,
    );
    let minted = harness.executor.execute(&mint_intent).await.unwrap();
    let mint_event = minted.first().cloned().unwrap();
    assert_eq!(mint_event.before.version, 0);
    assert_eq!(mint_event.after.version, 1);
    harness.index.apply(&mint_event, StateType::UniqueAsset);

    // §18.1: transfer (1 -> 2). `asset.transfer` is authority-required, so the
    // intent binds an authority proof and the FakeTrustGrant (allow) gate passes.
    let transfer_intent = validated_intent(
        &payload(
            "asset.transfer",
            "int_transfer_001",
            ALICE,
            RESOURCE,
            1,
            &inputs(&[("from_owner", json!(ALICE)), ("to_owner", json!(BOB))]),
        ),
        Some(sample_authority()),
    );
    let transferred = harness.executor.execute(&transfer_intent).await.unwrap();
    let transfer_event = transferred.first().cloned().unwrap();
    assert_eq!(transfer_event.before.version, 1);
    assert_eq!(transfer_event.after.version, 2);
    assert_eq!(
        transfer_event.after.state,
        json!({ "owner": BOB, "status": "active" })
    );
    harness.index.apply(&transfer_event, StateType::UniqueAsset);

    // §18.1: lock (2 -> 3), final owner BOB.
    let lock_intent = validated_intent(
        &payload(
            "asset.lock",
            "int_lock_001",
            BOB,
            RESOURCE,
            2,
            &BTreeMap::new(),
        ),
        None,
    );
    let locked = harness.executor.execute(&lock_intent).await.unwrap();
    let lock_event = locked.first().cloned().unwrap();
    assert_eq!(lock_event.before.version, 2);
    assert_eq!(lock_event.after.version, 3);
    assert_eq!(
        lock_event.after.state,
        json!({ "owner": BOB, "status": "locked" })
    );
    harness.index.apply(&lock_event, StateType::UniqueAsset);

    // Assemble the batch in execution order and form the commit (protocol §13.1).
    let events = vec![mint_event, transfer_event, lock_event];
    let batch = batch_from(&events);
    let builder = CommitBuilder::new(
        CommitScope::tenant(tenant()),
        1,
        executor_subject(),
        profile(),
        fixed_timestamp_placeholder(),
        None,
    );
    let previous_root = ContentDigest::new(*StateRoot::empty().as_bytes());
    let commit = builder
        .build(&batch, previous_root, &[], commit_id)
        .unwrap();

    // Determinism: the commit's next state root is a pure function of the
    // events' after-state set. A manually built accumulator reproduces it.
    let updates = state_root_updates(&events).unwrap();
    let mut accumulator = StateAccumulator::empty();
    accumulator.insert_batch(&updates).unwrap();
    assert_eq!(
        accumulator.root().as_bytes(),
        commit.next_state_root.as_bytes()
    );
    assert_eq!(
        compute_state_root(&updates).unwrap().as_bytes(),
        commit.next_state_root.as_bytes()
    );

    // Sign the commit body with the real Ed25519 key and verify it.
    let signed = sign_commit(&commit, &fixed_key(), key_id()).unwrap();
    verify_commit(&signed, &fixed_key().verifying_key()).unwrap();

    // Build the proof projection from the final (lock) event for the resource.
    let key = StateKey::for_resource(&tenant().0, RESOURCE);
    let lock = events.get(2).cloned().unwrap();
    let projection = StateProjection {
        tenant_id: tenant(),
        resource_id: ResourceId(String::from(RESOURCE)),
        state_type: StateType::UniqueAsset,
        version: lock.after.version,
        last_event_id: lock.event_id.clone(),
        last_commit_id: commit.commit_id,
        state_hash: lock.after.state_hash.clone(),
        state: lock.after.state,
    };

    Lifecycle {
        events,
        signed,
        accumulator,
        key,
        projection,
        final_owner: String::from(BOB),
    }
}

#[tokio::test]
async fn full_pipeline_submit_to_verify_end_to_end() {
    let lifecycle = run_lifecycle().await;

    // §16.3 / §29: prove inclusion of the transferred resource's leaf, assemble
    // the resource-state proof bundle, and verify it against the signed commit.
    let inclusion = lifecycle
        .accumulator
        .prove_inclusion(&lifecycle.key)
        .unwrap();
    let proof = build_state_proof(
        &lifecycle.projection,
        &lifecycle.signed,
        &inclusion,
        &Operation::new(String::from("asset.lock")).unwrap(),
        None,
        lifecycle.key,
    )
    .unwrap();

    assert_eq!(derive_state_key(&proof).unwrap(), lifecycle.key);
    assert!(
        verify_bundle(
            &proof,
            &lifecycle.signed,
            &fixed_key().verifying_key(),
            &lifecycle.key,
        )
        .is_ok()
    );

    // The final owner is BOB (transferred, then locked).
    assert!(verify_ownership(&proof, &lifecycle.final_owner).is_ok());
}

#[tokio::test]
async fn tampered_event_state_fails_verification() {
    let lifecycle = run_lifecycle().await;

    // Tamper the lock event's after-state: flip `status` back to `active`
    // (a forged state that never happened). Recompute its hash consistently.
    let mut tampered_event = lifecycle.events.get(2).cloned().unwrap();
    tampered_event.after.state = json!({ "owner": BOB, "status": "active" });
    tampered_event.after.state_hash = canonicalize_and_digest(&tampered_event.after.state).unwrap();

    let tampered_events = vec![
        lifecycle.events.first().cloned().unwrap(),
        lifecycle.events.get(1).cloned().unwrap(),
        tampered_event,
    ];
    let tampered_batch = batch_from(&tampered_events);
    let builder = CommitBuilder::new(
        CommitScope::tenant(tenant()),
        1,
        executor_subject(),
        profile(),
        fixed_timestamp_placeholder(),
        None,
    );
    let tampered_commit = builder
        .build(
            &tampered_batch,
            ContentDigest::new(*StateRoot::empty().as_bytes()),
            &[],
            commit_id,
        )
        .unwrap();

    // The tampered commit's next state root must differ from the genuine one.
    let genuine_root = lifecycle.signed.body.next_state_root.clone();
    assert_ne!(tampered_commit.next_state_root, genuine_root);

    let tampered_signed = sign_commit(&tampered_commit, &fixed_key(), key_id()).unwrap();

    // The genuine proof is pinned to the genuine root; verifying it against the
    // tampered commit envelope fails closed (state-root / commit-ref mismatch).
    let inclusion = lifecycle
        .accumulator
        .prove_inclusion(&lifecycle.key)
        .unwrap();
    let proof = build_state_proof(
        &lifecycle.projection,
        &lifecycle.signed,
        &inclusion,
        &Operation::new(String::from("asset.lock")).unwrap(),
        None,
        lifecycle.key,
    )
    .unwrap();
    assert!(
        verify_bundle(
            &proof,
            &tampered_signed,
            &fixed_key().verifying_key(),
            &lifecycle.key,
        )
        .is_err()
    );
}

#[tokio::test]
async fn absent_resource_non_membership_proof_verifies() {
    let lifecycle = run_lifecycle().await;

    // `asset:shield_001` was never minted, so it is absent from the tenant
    // accumulator. Prove its absence against the same signed commit.
    let absent_key = StateKey::for_resource(&tenant().0, "asset:shield_001");
    assert!(lifecycle.accumulator.prove_inclusion(&absent_key).is_none());

    let non_membership = lifecycle
        .accumulator
        .prove_non_membership(&absent_key)
        .unwrap();
    let absent_resource = ResourceId(String::from("asset:shield_001"));
    let bundle = build_non_membership_proof(
        &tenant(),
        &absent_resource,
        &absent_key,
        &lifecycle.signed,
        &non_membership,
    )
    .unwrap();

    assert!(
        verify_non_membership_bundle(
            &bundle,
            &lifecycle.signed,
            &fixed_key().verifying_key(),
            &absent_key,
        )
        .is_ok()
    );
}
