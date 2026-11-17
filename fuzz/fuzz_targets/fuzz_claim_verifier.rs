#![no_main]
//! Stateful canonical/historical verification through the public minimum reader.
//! Each input builds signed absence→presence checkpoints, then interleaves
//! independent expectations, corrupt tuples and a head advance during lookup.

use std::future::Future;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

use chrono::{DateTime, Utc};
use ed25519_dalek::SigningKey;
use libfuzzer_sys::fuzz_target;
use statechronicle::ports::commit_store::{CanonicalHead, CommitReadFuture, CommitReader};
use statechronicle_accumulator::key::StateKey;
use statechronicle_accumulator::sparse_merkle::{StateAccumulator, StateRoot, StateUpdate};
use statechronicle_commit::sign::sign_commit;
use statechronicle_core::canonicalize::canonicalize_and_digest;
use statechronicle_core::digest::{ContentDigest, hash_bytes};
use statechronicle_domain::commit::{Commit, CommitScope, ProfileId};
use statechronicle_domain::ids::{CommitId, EventId};
use statechronicle_domain::intent::{KeyId, Operation};
use statechronicle_domain::resource::ResourceId;
use statechronicle_domain::resource_state::ResourceState;
use statechronicle_domain::signed::Signed;
use statechronicle_domain::state::StateProjection;
use statechronicle_domain::state_type::StateType;
use statechronicle_domain::subject::SubjectId;
use statechronicle_domain::tenant::TenantId;
use statechronicle_proof::bundle::{build_non_membership_proof, build_state_proof};
use statechronicle_proof::claim::{ClaimProof, ClaimVerifier, ExpectedClaim, VerificationPoint};

// These in-memory futures are immediately ready; no runtime/dependency or unsafe
// dummy waker is needed. A pending future is a harness error, never a pass.
fn ready<T>(future: impl Future<Output = T>) -> T {
    let mut future = std::pin::pin!(future);
    match future
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
    {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("fixture reader unexpectedly suspended"),
    }
}

#[derive(Clone)]
struct Reader {
    tenant: TenantId,
    commits: [Signed<Commit>; 2],
    head: Arc<Mutex<Option<CanonicalHead>>>,
    advance_on_lookup: Arc<Mutex<Option<CanonicalHead>>>,
}

impl CommitReader for Reader {
    fn read_commit<'a>(
        &'a self,
        tenant: &'a TenantId,
        id: &'a CommitId,
    ) -> CommitReadFuture<'a, Option<Signed<Commit>>> {
        Box::pin(async move {
            if let Some(head) = self.advance_on_lookup.lock().unwrap().take() {
                *self.head.lock().unwrap() = Some(head);
            }
            Ok((tenant == &self.tenant)
                .then(|| {
                    self.commits
                        .iter()
                        .find(|c| &c.body.commit_id == id)
                        .cloned()
                })
                .flatten())
        })
    }

    fn read_canonical_head<'a>(
        &'a self,
        tenant: &'a TenantId,
    ) -> CommitReadFuture<'a, Option<CanonicalHead>> {
        Box::pin(async move {
            Ok((tenant == &self.tenant)
                .then(|| self.head.lock().unwrap().clone())
                .flatten())
        })
    }
}

fuzz_target!(|data: &[u8]| {
    let byte = |index| data.get(index).copied().unwrap_or(0);
    let tenant = TenantId(format!("fuzz.claim.{}", byte(0)));
    let resource = ResourceId(format!("asset:{}", byte(1)));
    let owner = SubjectId(format!("owner:{}", byte(2)));
    let signing = SigningKey::from_bytes(&std::array::from_fn(&byte));
    let trusted = signing.verifying_key();
    let mut other_seed = signing.to_bytes();
    other_seed[0] ^= 1;
    let untrusted = SigningKey::from_bytes(&other_seed).verifying_key();
    let key = StateKey::for_resource(&tenant.0, &resource.0);
    let mut accumulator = StateAccumulator::empty();
    let before_root = accumulator.root();
    let absent = accumulator.prove_non_membership(&key).unwrap();
    let state = ResourceState::from_legacy_json(
        StateType::UniqueAsset,
        serde_json::json!({"owner": owner.0, "status": "active"}),
    )
    .unwrap();
    let digest = canonicalize_and_digest(&state).unwrap();
    accumulator
        .insert_batch(&[StateUpdate::new(key, *digest.as_bytes())])
        .unwrap();
    let after_root = accumulator.root();
    let now = DateTime::<Utc>::from_timestamp(i64::from(byte(3)), 0).unwrap();
    let before_id = CommitId::new(String::from("cmt_claim_before")).unwrap();
    let after_id = CommitId::new(String::from("cmt_claim_after")).unwrap();
    let signed = |id: CommitId, previous, sequence, root: StateRoot| {
        sign_commit(
            &Commit::new(
                CommitScope::tenant(tenant.clone()),
                id,
                previous,
                sequence,
                1,
                hash_bytes(b"claim-events"),
                ContentDigest::new(*before_root.as_bytes()),
                ContentDigest::new(*root.as_bytes()),
                now,
                SubjectId(String::from("claim-executor")),
                ProfileId::new(String::from("statechronicle.profile.resource.v0")).unwrap(),
            ),
            &signing,
            KeyId::new(String::from("claim-key")).unwrap(),
        )
        .unwrap()
    };
    let before = signed(before_id.clone(), None, 1, before_root);
    let after = signed(after_id.clone(), Some(before_id.clone()), 2, after_root);
    let head_for = |commit_id: CommitId, sequence, root: StateRoot| CanonicalHead {
        commit_id,
        sequence,
        state_root: ContentDigest::new(*root.as_bytes()),
    };
    let old_head = head_for(before_id.clone(), 1, before_root);
    let new_head = head_for(after_id.clone(), 2, after_root);
    let reader = Reader {
        tenant: tenant.clone(),
        commits: [before.clone(), after.clone()],
        head: Arc::new(Mutex::new(Some(new_head.clone()))),
        advance_on_lookup: Arc::new(Mutex::new(None)),
    };
    let verifier = ClaimVerifier::new(Box::new(reader.clone()));
    let projection = StateProjection {
        tenant_id: tenant.clone(),
        resource_id: resource.clone(),
        state_type: StateType::UniqueAsset,
        version: 1,
        last_event_id: EventId::new(String::from("evt_claim_event")).unwrap(),
        last_commit_id: after_id.clone(),
        state_hash: digest,
        state,
    };
    let mut present = build_state_proof(
        &projection,
        &after,
        &accumulator.prove_inclusion(&key).unwrap(),
        &Operation::from_static("asset.create"),
        None,
        key,
    )
    .unwrap();
    let absence = build_non_membership_proof(&tenant, &resource, &key, &before, &absent).unwrap();
    let ownership = ExpectedClaim::ownership(tenant.clone(), resource.clone(), owner.clone());
    let missing = ExpectedClaim::absence(tenant.clone(), resource.clone());
    // Every input, including empty data, must reach genuine positive paths.
    assert!(
        ready(verifier.verify_current(ClaimProof::State(&present), &ownership, &trusted)).is_ok()
    );
    assert!(
        ready(verifier.verify_historical(
            ClaimProof::Absence(&absence),
            &missing,
            &before_id,
            &trusted
        ))
        .is_ok()
    );
    for choice in data.iter().take(32) {
        *reader.head.lock().unwrap() = Some(new_head.clone());
        *reader.advance_on_lookup.lock().unwrap() = None;
        match choice % 16 {
            0 => assert!(
                ready(verifier.verify_current(ClaimProof::State(&present), &ownership, &trusted))
                    .is_ok()
            ),
            1 => assert!(
                ready(verifier.verify_historical(
                    ClaimProof::Absence(&absence),
                    &missing,
                    &before_id,
                    &trusted
                ))
                .is_ok()
            ),
            2 => assert!(
                ready(verifier.verify_current(ClaimProof::Absence(&absence), &missing, &trusted))
                    .is_err()
            ),
            3..=5 => {
                let wrong = match choice % 16 {
                    3 => ExpectedClaim::ownership(
                        tenant.clone(),
                        ResourceId(format!("{}-other", resource.0)),
                        owner.clone(),
                    ),
                    4 => ExpectedClaim::ownership(
                        TenantId(format!("{}-other", tenant.0)),
                        resource.clone(),
                        owner.clone(),
                    ),
                    _ => ExpectedClaim::ownership(
                        tenant.clone(),
                        resource.clone(),
                        SubjectId(format!("{}-other", owner.0)),
                    ),
                };
                assert!(
                    ready(verifier.verify_current(ClaimProof::State(&present), &wrong, &trusted))
                        .is_err()
                );
                assert!(
                    ready(verifier.verify_historical(
                        ClaimProof::State(&present),
                        &wrong,
                        &after_id,
                        &trusted
                    ))
                    .is_err()
                );
            }
            6 => {
                assert!(
                    ready(verifier.verify_current(
                        ClaimProof::State(&present),
                        &ownership,
                        &untrusted
                    ))
                    .is_err()
                );
                assert!(
                    ready(verifier.verify_historical(
                        ClaimProof::Absence(&absence),
                        &missing,
                        &before_id,
                        &untrusted
                    ))
                    .is_err()
                );
            }
            7 => assert!(
                ready(verifier.verify_historical(
                    ClaimProof::State(&present),
                    &ownership,
                    &before_id,
                    &trusted
                ))
                .is_err(),
                "claim.historical.pin"
            ),
            8 => {
                let mut corrupt = present.clone();
                corrupt.state_inclusion_proof.leaf_hash = hash_bytes(b"corrupt-claim");
                assert!(
                    ready(verifier.verify_current(
                        ClaimProof::State(&corrupt),
                        &ownership,
                        &trusted
                    ))
                    .is_err()
                );
            }
            9..=12 => {
                let mut head = new_head.clone();
                match choice % 16 {
                    10 => head.state_root = hash_bytes(b"wrong-head-root"),
                    11 => head.sequence += 1,
                    12 => head.commit_id = before_id.clone(),
                    _ => {}
                }
                *reader.head.lock().unwrap() = (choice % 16 != 9).then_some(head);
                assert!(
                    ready(verifier.verify_current(
                        ClaimProof::State(&present),
                        &ownership,
                        &trusted
                    ))
                    .is_err()
                );
            }
            13 => {
                *reader.head.lock().unwrap() = Some(old_head.clone());
                *reader.advance_on_lookup.lock().unwrap() = Some(new_head.clone());
                assert!(
                    ready(verifier.verify_current(
                        ClaimProof::Absence(&absence),
                        &missing,
                        &trusted
                    ))
                    .is_err(),
                    "claim.current.head_advance"
                );
            }
            14 => {
                *reader.head.lock().unwrap() = None;
                assert!(
                    ready(verifier.verify_historical(
                        ClaimProof::State(&present),
                        &ownership,
                        &after_id,
                        &trusted
                    ))
                    .is_ok()
                );
            }
            _ => {
                let verified = ready(verifier.verify_current(
                    ClaimProof::State(&present),
                    &ownership,
                    &trusted,
                ))
                .unwrap();
                assert_eq!(verified.point(), VerificationPoint::Current);
                assert_eq!(verified.expectation(), &ownership);
                assert_eq!(verified.commit().commit_id, after_id);
                let original = present.claimed_state.clone();
                present.claimed_state = ResourceState::from_legacy_json(
                    StateType::UniqueAsset,
                    serde_json::json!({"owner":"changed", "status":"active"}),
                )
                .unwrap();
                assert_eq!(verified.state(), Some(&original));
                present.claimed_state = original;
            }
        }
    }
});
