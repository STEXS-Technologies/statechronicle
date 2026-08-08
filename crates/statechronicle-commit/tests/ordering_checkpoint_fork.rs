//! Integration tests for ordering, global checkpoints, and fork semantics.
//!
//! Exercises the public API over real domain types: the full global
//! checkpoint lifecycle (build from two tenants with real `StateRoot`s via the
//! accumulator, sign, verify, tamper → fail), fork detection between two
//! candidate commits claiming the same parent and sequence, chain continuity
//! across a real three-commit chain, and event-rewrite rejection.

#![allow(
    clippy::panic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::unwrap_in_result,
    clippy::panic_in_result_fn,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use chrono::{DateTime, Utc};
use ed25519_dalek::SigningKey;

use statechronicle_accumulator::key::StateKey;
use statechronicle_accumulator::sparse_merkle::{StateAccumulator, StateRoot, StateUpdate};

use statechronicle_core::canonicalize::canonicalize_and_digest;
use statechronicle_core::digest::hash_bytes;

use statechronicle_domain::commit::{Commit, CommitScope, ProfileId};
use statechronicle_domain::event::{Event, StateCommitment};
use statechronicle_domain::ids::{CommitId, EventId, IntentId};
use statechronicle_domain::intent::{KeyId, Operation};
use statechronicle_domain::resource::ResourceId;
use statechronicle_domain::subject::SubjectId;
use statechronicle_domain::tenant::TenantId;

use statechronicle_commit::batch::CommitBatch;
use statechronicle_commit::builder::CommitBuilder;
use statechronicle_commit::checkpoint::{
    TenantRootEntry, build_global_checkpoint, sign_global_checkpoint, verify_global_checkpoint,
};
use statechronicle_commit::error::CommitError;
use statechronicle_commit::fork::{check_chain_continuity, detect_fork, validate_no_event_rewrite};
use statechronicle_commit::roots::{compute_state_root, state_root_updates};

const FIXED_SEED: [u8; 32] = [42u8; 32];

fn tenant() -> TenantId {
    TenantId(String::from("acme.game.alpha"))
}

fn executor() -> SubjectId {
    SubjectId(String::from("service:statechronicle.example.net"))
}

fn profile() -> ProfileId {
    ProfileId::new(String::from("statechronicle.profile.resource.v0")).unwrap()
}

fn timestamp() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-07-14T00:00:07Z")
        .unwrap()
        .with_timezone(&Utc)
}

fn fixed_key() -> SigningKey {
    SigningKey::from_bytes(&FIXED_SEED)
}

fn checkpoint_key_id() -> KeyId {
    KeyId::new(String::from("did:key:z6Mk...#global-checkpoint")).unwrap()
}

fn commitment(version: u64, owner: &str) -> StateCommitment {
    let state = serde_json::json!({ "owner": owner, "status": "active" });
    StateCommitment {
        version,
        state_hash: canonicalize_and_digest(&state).unwrap(),
        state,
    }
}

fn event(id: &str, resource: &str, owner: &str) -> Event {
    Event::new(
        tenant(),
        EventId::new(format!("evt_{id}")).unwrap(),
        IntentId::new(format!("int_{id}")).unwrap(),
        Operation::new(String::from("asset.transfer")).unwrap(),
        ResourceId(String::from(resource)),
        SubjectId(String::from("account:example:player_123")),
        commitment(1, "account:example:player_000"),
        commitment(2, owner),
        None,
        executor(),
        timestamp(),
    )
}

fn batch_from(events: &[Event]) -> CommitBatch {
    let mut batch = CommitBatch::new(CommitScope::tenant(tenant()));
    for event in events {
        batch.add_event(event.clone()).unwrap();
    }
    batch
}

fn commit_id(sequence: u64) -> Result<CommitId, CommitError> {
    CommitId::new(format!("cmt_{sequence:020}")).map_err(CommitError::from)
}

/// Builds a "real" state root for a tenant by inserting one update into an
/// accumulator.
fn real_root(tenant_id: &TenantId, resource: &str, digest: [u8; 32]) -> StateRoot {
    let mut accumulator = StateAccumulator::empty();
    let key = StateKey::for_resource(tenant_id.0.as_str(), resource);
    accumulator
        .insert_batch(&[StateUpdate::new(key, digest)])
        .unwrap();
    accumulator.root()
}

// ---------------------------------------------------------------------------
// Global checkpoint lifecycle.
// ---------------------------------------------------------------------------

#[test]
fn global_checkpoint_lifecycle() {
    // Two tenants with real accumulator-derived state roots.
    let alpha = tenant();
    let beta = TenantId(String::from("acme.marketplace"));
    let alpha_root = real_root(
        &alpha,
        "asset:sword",
        hash_bytes(b"alpha-state").as_bytes().to_owned(),
    );
    let beta_root = real_root(
        &beta,
        "asset:shield",
        hash_bytes(b"beta-state").as_bytes().to_owned(),
    );

    let entries = vec![
        TenantRootEntry {
            tenant_id: alpha,
            commit_id: CommitId::new(String::from("cmt_alpha_tip_001")).unwrap(),
            state_root: alpha_root,
        },
        TenantRootEntry {
            tenant_id: beta,
            commit_id: CommitId::new(String::from("cmt_beta_tip_001")).unwrap(),
            state_root: beta_root,
        },
    ];
    let checkpoint = build_global_checkpoint(entries, 55102, timestamp(), executor()).unwrap();
    assert_eq!(checkpoint.schema, "statechronicle.global_checkpoint.v0");
    assert_eq!(checkpoint.sequence, 55102);
    assert_eq!(checkpoint.tenant_roots.len(), 2);

    // Sign, verify, tamper → fail.
    let key = fixed_key();
    let signed = sign_global_checkpoint(&checkpoint, &key, checkpoint_key_id()).unwrap();
    assert!(verify_global_checkpoint(&signed, &key.verifying_key()).is_ok());

    let mut tampered = signed;
    tampered.body.sequence = tampered.body.sequence.wrapping_add(1);
    assert!(matches!(
        verify_global_checkpoint(&tampered, &key.verifying_key()),
        Err(CommitError::Core(_))
    ));
}

// ---------------------------------------------------------------------------
// Fork detection and chain continuity.
// ---------------------------------------------------------------------------

/// A minimal commit for fork/continuity tests.
fn raw_commit(id: &str, parent: Option<&str>, sequence: u64) -> Commit {
    Commit::new(
        CommitScope::tenant(tenant()),
        CommitId::new(String::from(id)).unwrap(),
        parent.map(|value| CommitId::new(String::from(value)).unwrap()),
        sequence,
        1,
        hash_bytes(b"event-root"),
        hash_bytes(b"previous-root"),
        hash_bytes(b"next-root"),
        timestamp(),
        executor(),
        profile(),
    )
}

#[test]
fn fork_detection_rejects_two_heads() {
    let previous = raw_commit("cmt_shared_parent", None, 1);
    let candidate_a = raw_commit("cmt_head_a", Some("cmt_shared_parent"), 2);
    let candidate_b = raw_commit("cmt_head_b", Some("cmt_shared_parent"), 2);
    let error = detect_fork(&previous, &candidate_a, &candidate_b).unwrap_err();
    assert!(matches!(
        error,
        CommitError::ForkDetected { parent, sequence }
        if parent == "cmt_shared_parent" && sequence == 2
    ));

    // The same commit presented twice is not a fork.
    assert!(detect_fork(&previous, &candidate_a, &candidate_a).is_ok());
}

#[test]
fn chain_continuity_across_three_real_commits() {
    // Build a real 3-commit chain through the builder + roots.
    let first_events = vec![event("sword", "asset:sword", "alice")];
    let second_events = vec![event("shield", "asset:shield", "bob")];
    let third_events = vec![event("potion", "asset:potion", "carol")];

    let genesis_root = hash_bytes(b"genesis");
    let builder = CommitBuilder::new(
        CommitScope::tenant(tenant()),
        1,
        executor(),
        profile(),
        timestamp(),
        None,
    );
    let commit1 = builder
        .build(&batch_from(&first_events), genesis_root, &[], || {
            commit_id(1)
        })
        .unwrap();
    let updates1 = state_root_updates(&first_events).unwrap();
    assert_eq!(
        commit1.next_state_root.as_bytes(),
        compute_state_root(&updates1).unwrap().as_bytes()
    );

    let builder2 = CommitBuilder::new(
        CommitScope::tenant(tenant()),
        2,
        executor(),
        profile(),
        timestamp(),
        Some(commit1.commit_id.clone()),
    );
    let commit2 = builder2
        .build(
            &batch_from(&second_events),
            commit1.next_state_root.clone(),
            &updates1,
            || commit_id(2),
        )
        .unwrap();
    let updates2 = state_root_updates(&second_events).unwrap();
    let mut combined = updates1;
    combined.extend_from_slice(&updates2);
    combined.sort_by_key(|a| a.key);

    let builder3 = CommitBuilder::new(
        CommitScope::tenant(tenant()),
        3,
        executor(),
        profile(),
        timestamp(),
        Some(commit2.commit_id.clone()),
    );
    let commit3 = builder3
        .build(
            &batch_from(&third_events),
            commit2.next_state_root.clone(),
            &combined,
            || commit_id(3),
        )
        .unwrap();

    // The chain is continuous.
    assert!(check_chain_continuity(&commit1, &commit2).is_ok());
    assert!(check_chain_continuity(&commit2, &commit3).is_ok());

    // A mis-linked successor fails closed.
    let broken = raw_commit("cmt_broken", Some("cmt_other"), 4);
    assert!(matches!(
        check_chain_continuity(&commit2, &broken),
        Err(CommitError::ChainGap { .. })
    ));
    let wrong_sequence = raw_commit("cmt_wrong_seq", Some(commit2.commit_id.as_str()), 9);
    assert!(matches!(
        check_chain_continuity(&commit2, &wrong_sequence),
        Err(CommitError::SequenceMismatch { expected, actual })
        if expected == 3 && actual == 9
    ));
}

// ---------------------------------------------------------------------------
// Event rewrite rejection.
// ---------------------------------------------------------------------------

#[test]
fn event_rewrite_is_rejected() {
    let accepted = event("sword", "asset:sword", "alice");
    let identical = event("sword", "asset:sword", "alice");
    assert!(validate_no_event_rewrite(&accepted, &identical).is_ok());

    let rewritten = event("sword", "asset:sword", "bob");
    let error = validate_no_event_rewrite(&accepted, &rewritten).unwrap_err();
    assert!(matches!(
        error,
        CommitError::EventRewrite { event_id } if event_id == "evt_sword"
    ));
}
