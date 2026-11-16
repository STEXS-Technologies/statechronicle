//! Read original-release history and goldens; never regenerate expectations here.
use serde::Deserialize;
use statechronicle::accumulator::key::StateKey;
use statechronicle::commit::roots::state_root_updates;
use statechronicle::core::canonicalize::canonicalize;
use statechronicle::domain::proof::{NonMembershipProofBundle, ResourceStateProof};
use statechronicle::proof::verify::{verify_bundle, verify_non_membership_bundle};
use statechronicle::{Commit, Event, Signed, StateAccumulator, StateProjection};

#[derive(Deserialize)]
struct Corpus {
    producer_commit: String,
    events: Vec<Event>,
    signed_commit: Signed<Commit>,
    projection: StateProjection,
    state_proof: ResourceStateProof,
    absence_proof: NonMembershipProofBundle,
    commit_signing_bytes: Vec<u8>,
    event_bytes: Vec<u8>,
    state_proof_bytes: Vec<u8>,
    absence_proof_bytes: Vec<u8>,
}

#[test]
fn original_history_signatures_roots_and_proof_bytes_remain_compatible() {
    let corpus: Corpus = serde_json::from_str(include_str!("../../../wire/history.json")).unwrap();
    assert_eq!(
        corpus.producer_commit,
        "c418e98b91720188158c531cd5c2c4b8ab71cbcf"
    );
    assert_eq!(
        canonicalize(&corpus.signed_commit.body).unwrap(),
        corpus.commit_signing_bytes
    );
    assert_eq!(canonicalize(&corpus.events).unwrap(), corpus.event_bytes);
    assert_eq!(
        canonicalize(&corpus.state_proof).unwrap(),
        corpus.state_proof_bytes
    );
    assert_eq!(
        canonicalize(&corpus.absence_proof).unwrap(),
        corpus.absence_proof_bytes
    );
    let mut accumulator = StateAccumulator::empty();
    accumulator
        .insert_batch(&state_root_updates(&corpus.events).unwrap())
        .unwrap();
    assert_eq!(
        accumulator.root().as_bytes(),
        corpus.signed_commit.body.next_state_root.as_bytes()
    );
    assert_eq!(
        corpus.projection.state,
        serde_json::json!({"owner":"account:bob", "status":"active"})
    );
    // Signer and requested key come from this verifier, not the supplied bundle.
    let trusted = ed25519_dalek::SigningKey::from_bytes(&[42; 32]).verifying_key();
    let expected = StateKey::for_resource("acme.game.alpha", "asset:wire");
    verify_bundle(
        &corpus.state_proof,
        &corpus.signed_commit,
        &trusted,
        &expected,
    )
    .unwrap();
    let absent = StateKey::for_resource("acme.game.alpha", "asset:absent");
    verify_non_membership_bundle(
        &corpus.absence_proof,
        &corpus.signed_commit,
        &trusted,
        &absent,
    )
    .unwrap();
    assert!(
        verify_bundle(
            &corpus.state_proof,
            &corpus.signed_commit,
            &trusted,
            &absent
        )
        .is_err()
    );
    assert!(
        verify_non_membership_bundle(
            &corpus.absence_proof,
            &corpus.signed_commit,
            &trusted,
            &expected
        )
        .is_err()
    );
    let wrong_signer = ed25519_dalek::SigningKey::from_bytes(&[43; 32]).verifying_key();
    assert!(
        verify_bundle(
            &corpus.state_proof,
            &corpus.signed_commit,
            &wrong_signer,
            &expected
        )
        .is_err()
    );
}
