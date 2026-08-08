//! Run: `cargo run -p statechronicle --example proofs`
//!
//! Proof building and verification over a mint → transfer → lock lane: a
//! resource-state proof, an ownership proof (owner BOB), a non-membership proof
//! for an absent asset, and a fail-closed rejection when a genuine proof is
//! checked against a tampered commit.

#![allow(
    clippy::panic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::type_complexity
)]

mod common;

use serde_json::json;

use statechronicle::accumulator::key::StateKey;
use statechronicle::domain::authority::AuthorityProof;
use statechronicle::domain::ids::IntentId;
use statechronicle::domain::intent::{Intent, Nonce, Operation};
use statechronicle::domain::resource::ResourceId;
use statechronicle::domain::state::StateProjection;
use statechronicle::domain::state_type::StateType;
use statechronicle::domain::subject::SubjectId;
use statechronicle::intent::validated::ValidatedIntent;
use statechronicle::proof::bundle::{
    build_non_membership_proof, build_ownership_proof, build_state_proof,
};
use statechronicle::proof::verify::{
    verify_bundle, verify_non_membership_bundle, verify_ownership,
};

use common::Harness;

const ALICE: &str = "account:example:player_123";
const BOB: &str = "account:example:player_456";
const RESOURCE: &str = "asset:sword_001";

/// Builds a signed `asset.*` intent via `Intent::builder()` + `harness.sign`.
#[allow(clippy::too_many_arguments)]
fn signed(
    harness: &Harness,
    id: &str,
    op: &'static str,
    version: u64,
    inputs: &[(&str, serde_json::Value)],
    authority: Option<AuthorityProof>,
) -> ValidatedIntent {
    let mut b = Intent::builder()
        .tenant(harness.tenant())
        .intent_id(IntentId::new(format!("int_{id}")).unwrap())
        .operation(Operation::from_static(op))
        .actor(SubjectId(String::from(ALICE)))
        .resource(ResourceId(String::from(RESOURCE)))
        .state_type(StateType::UniqueAsset)
        .expected_version(version)
        .created_at(harness.now())
        .nonce(Nonce::from_bytes(vec![0]).unwrap());
    for (k, v) in inputs {
        b = b.input(k, v.clone());
    }
    harness.sign(b.build().unwrap(), authority)
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let harness = Harness::new();
    let mut events: Vec<statechronicle::domain::event::Event> = Vec::new();

    println!("== proofs: state, ownership, and non-membership proofs ==");

    // mint(ALICE): version 0 -> 1.
    let minted = harness
        .run(
            &signed(
                &harness,
                "pf_mint",
                "asset.mint",
                0,
                &[("to_owner", json!(ALICE))],
                None,
            ),
            StateType::UniqueAsset,
        )
        .await;
    events.push(minted);

    // transfer(ALICE -> BOB, authority): 1 -> 2.
    let transferred = harness
        .run(
            &signed(
                &harness,
                "pf_transfer",
                "asset.transfer",
                1,
                &[("from_owner", json!(ALICE)), ("to_owner", json!(BOB))],
                Some(harness.authority()),
            ),
            StateType::UniqueAsset,
        )
        .await;
    events.push(transferred);

    // lock(BOB): 2 -> 3.
    let locked = harness
        .run(
            &signed(&harness, "pf_lock", "asset.lock", 2, &[], None),
            StateType::UniqueAsset,
        )
        .await;
    events.push(locked);
    println!(
        "mint -> transfer -> lock; final state {}",
        events[2].after.state
    );

    // Form + sign the commit; reproduce its root in an accumulator.
    let (signed, accumulator) = harness.commit_events(&events);
    let key = StateKey::for_resource(&harness.tenant().0, RESOURCE);
    let projection = StateProjection {
        tenant_id: harness.tenant(),
        resource_id: ResourceId(String::from(RESOURCE)),
        state_type: StateType::UniqueAsset,
        version: events[2].after.version,
        last_event_id: events[2].event_id.clone(),
        last_commit_id: signed.body.commit_id.clone(),
        state_hash: events[2].after.state_hash.clone(),
        state: events[2].after.state.clone(),
    };

    // State proof + verify_bundle.
    let inclusion = accumulator.prove_inclusion(&key).unwrap();
    let op_lock = Operation::from_static("asset.lock");
    let state_proof =
        build_state_proof(&projection, &signed, &inclusion, &op_lock, None, key).unwrap();
    let fixed_key = common::fixed_key();
    assert!(verify_bundle(&state_proof, &signed, &fixed_key.verifying_key(), &key).is_ok());
    println!("build_state_proof + verify_bundle -> OK");

    // Ownership proof: BOB owns the locked asset.
    let ownership = build_ownership_proof(
        &projection,
        &signed,
        &inclusion,
        &op_lock,
        &SubjectId(String::from(BOB)),
        None,
        key,
    )
    .unwrap();
    assert!(verify_ownership(&ownership, BOB).is_ok());
    println!("build_ownership_proof + verify_ownership(BOB) -> OK");

    // Non-membership: an asset that was never minted is absent from the tree.
    let absent_resource = "asset:shield_001";
    let absent_key = StateKey::for_resource(&harness.tenant().0, absent_resource);
    assert!(accumulator.prove_inclusion(&absent_key).is_none());
    let non_membership = accumulator.prove_non_membership(&absent_key).unwrap();
    let bundle = build_non_membership_proof(
        &harness.tenant(),
        &ResourceId(String::from(absent_resource)),
        &absent_key,
        &signed,
        &non_membership,
    )
    .unwrap();
    assert!(
        verify_non_membership_bundle(&bundle, &signed, &fixed_key.verifying_key(), &absent_key)
            .is_ok()
    );
    println!("non-membership for {absent_resource} -> verifies");

    // Tamper the commit envelope; the genuine state proof now fails closed.
    let mut tampered = signed;
    tampered.body.sequence = tampered.body.sequence.wrapping_add(1);
    assert!(verify_bundle(&state_proof, &tampered, &fixed_key.verifying_key(), &key).is_err());
    println!("state proof vs tampered commit -> rejected");

    println!("proofs: OK");
}
